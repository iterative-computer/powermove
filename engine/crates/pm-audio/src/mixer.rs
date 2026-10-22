//! The mixer: clip voices → per-layer strips → master, in f32 at the
//! project's `audioSampleRate`.
//!
//! [`Mixer::render`] is a pure function of the document, the composition,
//! and the project sample range: no state carries from one render to the
//! next except caches. A render that starts at sample n returns what a
//! render through n returns there, so seeking, looping, and pre-roll are
//! deterministic and the preview equals the export.
//!
//! # Automation
//!
//! Gain is applied per sample. Animated inputs (levels, `on` switches,
//! fades, remaps, speed) are evaluated through `pm_eval` at control points
//! every [`CONTROL`] samples, aligned to absolute sample indexes, and
//! interpolated linearly between them; fades are computed per sample from
//! the clip's local time. Linear time maps (no remap, no speed) place every
//! sample exactly; other maps interpolate the source position between
//! control points.
//!
//! # Pitch
//!
//! A time-mapped voice whose clip preserves pitch (the default) plays
//! through a time stretch ([`crate::stretch`]), whose deterministic
//! pre-roll keeps the render a function of the sample range and of where
//! rendering starts. Clips that follow speed, and constant rates within
//! [`crate::stretch::unity_band`] of 1, resample (pitch follows the speed).

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use pm_eval::Evaluator;
use pm_eval::retime::RetimeCache;
use pm_media::audio::Speaker;
use pm_model::Document;

use crate::error::Result;
use crate::graph::{MixGraph, Point, VoicePlan};
use crate::kernel::{self, Bank};
use crate::layout;
use crate::meter::{Meters, Tap};
use crate::source::{Source, SourceCache};
use crate::stretch::{self, MapPoint, StretchRun};

/// Samples between automation control points (3 kHz at 48 kHz).
pub const CONTROL: i64 = 16;

/// A live edit during playback (a fader drag, a mute, a monitor solo) glides
/// from what played into the new mix over this long instead of stepping, as
/// today's engine does (`RETUNE_GLIDE` in `legacy/core/audio.ts`): hard steps
/// on a sustained sound are audible as clicks and zipper noise.
pub const RETUNE_GLIDE_MS: u32 = 12;

/// Control points per memoized read bound of a time-mapped voice
/// ([`Mixer::source_spans`]): 4096 samples.
pub const READ_GROUP: i64 = 256;

/// Memoized read bounds kept before they are dropped and evaluated again.
const MAX_READ_GROUPS: usize = 1 << 16;

/// Source sample spans `[lo, hi)`.
type Spans = Arc<[(i64, i64)]>;

/// Retune glides kept at once; an older one is cut short (a fader dragged at
/// 60 Hz starts a glide every 16.7 ms, so two overlap at most).
const MAX_GLIDES: usize = 4;

/// The mix bus.
#[derive(Clone, Debug, PartialEq)]
pub struct MixFormat {
    pub sample_rate: u32,
    pub speakers: Vec<Speaker>,
}

impl MixFormat {
    pub fn stereo(sample_rate: u32) -> MixFormat {
        MixFormat {
            sample_rate,
            speakers: layout::stereo(),
        }
    }
    pub fn channels(&self) -> usize {
        self.speakers.len()
    }
}

/// The project audio rate (`audioSampleRate`, section 6), 48 kHz when the
/// document does not set one or sets one outside 8 to 192 kHz.
pub fn project_sample_rate(doc: &Document) -> u32 {
    let rate = pm_eval::node::to_number(pm_eval::node::member(doc.root(), "audioSampleRate"));
    if rate.is_finite() && rate.fract() == 0.0 && (8_000.0..=192_000.0).contains(&rate) {
        rate as u32
    } else {
        48_000
    }
}

/// Per-render choices.
#[derive(Clone, Debug, Default)]
pub struct RenderOptions {
    /// Decode missing source blocks on this thread (export, offline). When
    /// false (playback), missing blocks are queued and play as silence.
    pub wait: bool,
    /// Commit missing playback input as silence instead of retrying it.
    /// Offline renders leave this false and wait for all input.
    pub drop_starved: bool,
    /// Per-voice playback deadlines that have expired. Other missing voices
    /// still retry; a dropped voice never spends another voice's budget.
    pub drop_voices: HashSet<String>,
    /// Strips heard (the mixer's monitor solo; never part of an export).
    /// `None` or empty: every strip.
    pub monitor_solo: Option<HashSet<String>>,
    /// Update the level taps.
    pub meters: bool,
    /// Resample stretched voices instead (their pitch follows the speed):
    /// reverse shuttle renders each chunk on its own, which a stretch's
    /// pre-roll would make far too costly.
    pub no_stretch: bool,
}

/// What a render met.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderReport {
    /// Voices that sounded somewhere in the range.
    pub voices: usize,
    /// Voices that had source blocks missing (playback only).
    pub starved: usize,
    /// IDs of voices missing input, including outgoing retune voices, each
    /// once.
    pub starved_voices: Vec<String>,
    /// Missing voices whose own deadline has not expired. No PCM or meter /
    /// retune progress may commit until this is empty.
    pub waiting_voices: Vec<String>,
    /// IDs of voices that sounded with all their input on a strip the
    /// monitor solo lets through, in the current or an outgoing retune mix,
    /// each once. Unlike `voices - starved`, a glide's outgoing copies of the
    /// same voices do not count twice. These diagnostics describe what is
    /// actually heard; A6 bounds every mid-run wait by the ring reserve.
    pub heard_whole: Vec<String>,
    /// Project output frames silenced for missing input, summed over
    /// voices. A voice a retune glide renders in several mixes counts once
    /// (its largest drop), as in `dropped_by_voice`.
    pub dropped_voice_frames: u64,
    /// Output frames silenced for missing input, per voice id, each once.
    pub dropped_by_voice: Vec<(String, u64)>,
    /// Errors decoding or evaluating, one per voice.
    pub errors: Vec<(String, String)>,
}

#[derive(Clone)]
enum Reader {
    /// Source rate equals project rate and the map is linear: integer copy.
    Copy { shift: i64 },
    /// Linear map at a rational ratio: exact phases.
    Grid {
        bank: Arc<Bank>,
        /// Position units (1/phases of a source sample) per output sample.
        step: i128,
        /// Position of output sample 0, in units.
        origin: i128,
        phases: i128,
    },
    /// Any other map: free positions.
    Free,
}

struct Voice {
    reader: Reader,
    matrix: Vec<Vec<f32>>,
    strip: usize,
    /// The pitch-preserving stretch of a time-mapped voice, shared with a
    /// glide's outgoing mix while the map is the same.
    stretch: Option<Arc<std::sync::Mutex<StretchRun>>>,
    /// Reads the source at the project rate (the stretch's input).
    project_reader: Option<Reader>,
    preparing: Option<i64>,
    fixed: Option<crate::graph::StaticMap>,
    prefill: Option<Prefill>,
}

/// Whether a voice plays through the time stretch.
pub fn stretches(plan: &VoicePlan, project: u32) -> bool {
    plan.pitch == crate::conform::Pitch::Preserve
        && plan.offset.is_none()
        && plan
            .constant_rate
            .is_none_or(|r| stretch::stretches_constant(r, project))
}

/// A mix being left by a retune glide: the state it played with and how far
/// the glide into the next state has come.
struct Outgoing {
    doc: Arc<Document>,
    graph: Arc<MixGraph>,
    voices: Vec<Voice>,
    master_gain: f32,
    retime: RetimeCache,
    solo: Option<HashSet<String>>,
    /// Samples rendered since the glide began.
    done: usize,
}

/// Mixes one composition.
pub struct Mixer {
    format: MixFormat,
    doc: Arc<Document>,
    graph: Arc<MixGraph>,
    ev: Evaluator,
    retime: RetimeCache,
    cache: Arc<SourceCache>,
    voices: Vec<Voice>,
    taps: Vec<Tap>,
    master_tap: Tap,
    master_gain: f32,
    // Scratch.
    gains: Vec<f32>,
    locals: Vec<f64>,
    positions: Vec<f64>,
    /// Source samples per output sample, per sample (free reads).
    steps: Vec<f64>,
    src: Vec<Vec<f32>>,
    src_valid: Vec<bool>,
    voice_dropped_frames: u64,
    voice_out: Vec<Vec<f32>>,
    strips: Vec<Vec<Vec<f32>>>,
    kernel: Vec<f32>,
    points: Vec<Point>,
    /// Retune glides in progress, oldest first; each fades into the next
    /// (the last into the current state).
    outgoing: Vec<Outgoing>,
    glide_len: usize,
    glide_acc: Vec<Vec<f32>>,
    glide_tmp: Vec<Vec<f32>>,
    /// Strip buffers of the outgoing mixes (the current mix's stay in
    /// `strips` for its meters).
    glide_strips: Vec<Vec<Vec<f32>>>,
    /// Read bounds of time-mapped voices, per (voice, control-point group).
    reads: HashMap<(usize, i64), Spans>,
    /// Paused playhead/work-area vocoders. Separate histories keep each
    /// prepared start identical to a fresh offline render at that point.
    prepared: HashMap<i64, Prepared>,
    standard_content: bool,
}

// A normal ring's worth of complete PCM covers independent priming.
// Keep the time reserve across project rates. Reuse applies only to
// exactly equal constant signals, before each voice's gain and strip.
const HEAD_FRAMES: usize = 4096;
fn head_frames(rate: u32) -> usize {
    (HEAD_FRAMES * rate as usize)
        .div_ceil(48_000)
        .next_multiple_of(crate::ring::CHUNK)
}
/// Output a start's own job renders past its first chunk, so the producer
/// queues the start's opening callbacks from vocoder output already in the
/// history instead of running sixteen vocoder steps serially between the
/// first and second ring chunks.
const LEAD_FRAMES: usize = 1024;
fn lead_frames(rate: u32) -> usize {
    (LEAD_FRAMES * rate as usize)
        .div_ceil(48_000)
        .next_multiple_of(crate::ring::CHUNK)
}
struct Head {
    start: i64,
    ready_us: u64,
    pcm: Vec<Vec<f32>>,
}
impl Head {
    fn covers(&self, at: i64, frames: usize) -> bool {
        at >= self.start && (at - self.start) as usize + frames <= self.pcm[0].len()
    }
}
struct HeadSlot {
    pcm: std::sync::Mutex<Option<Arc<Head>>>,
    available: Arc<AtomicBool>,
}
impl HeadSlot {
    fn new() -> Self {
        Self {
            pcm: std::sync::Mutex::new(None),
            available: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl std::ops::Deref for HeadSlot {
    type Target = std::sync::Mutex<Option<Arc<Head>>>;
    fn deref(&self) -> &Self::Target {
        &self.pcm
    }
}
type SharedHead = Arc<HeadSlot>;
#[derive(Clone)]
struct Prefill {
    head: Arc<Head>,
    finished: Arc<AtomicBool>,
}

/// Detached priming work. It never touches live histories, meters or the transport.
pub(crate) struct PrepareVoice {
    pub(crate) start: i64,
    pub(crate) vi: usize,
    target: i64,
    doc: Arc<Document>,
    graph: Arc<MixGraph>,
    cache: Arc<SourceCache>,
    rate: u32,
    reader: Reader,
    project_reader: Reader,
    run: Arc<std::sync::Mutex<StretchRun>>,
    head: Option<SharedHead>,
    pub(crate) gate: Option<Arc<AtomicBool>>,
    pub(crate) critical: Arc<AtomicBool>,
    started: Arc<AtomicBool>,
    /// Set by whoever ran this job once its history is released. A head's
    /// consumer waits on it alone: never on the job's result reaching the
    /// producer, which a dropped or superseded start entry would discard.
    finished: Arc<AtomicBool>,
}
impl PrepareVoice {
    pub(crate) fn history(&self) -> Arc<std::sync::Mutex<StretchRun>> {
        self.run.clone()
    }
    /// Claims this job for a worker or paused preparation. False when another
    /// thread already owns its history.
    pub(crate) fn claim(&self) -> bool {
        !self.started.swap(true, Ordering::AcqRel)
    }
    pub(crate) fn into_task(self, ev: Evaluator, static_content: bool) -> PrepareTask {
        PrepareTask {
            work: self,
            ev,
            static_content,
            retime: RetimeCache::new(),
            fixed: None,
            initialized: false,
            first: 0,
            a: 0,
            last: 0,
            stage: PrepareStage::Join,
            src: Vec::new(),
            out: Vec::new(),
            terminal: false,
            #[cfg(test)]
            last_slice_boundaries: 0,
            #[cfg(test)]
            before_history_lock: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn run(
        self,
        ev: &mut Evaluator,
        static_content: bool,
        cancelled: impl Fn() -> bool,
    ) -> Result<bool> {
        // Direct test callers use the same state machine without fairness
        // yields; preserve their evaluator and all evaluation memoization.
        let owned = std::mem::replace(ev, Evaluator::new());
        let mut task = self.into_task(owned, static_content);
        let result = match task.step_with(&cancelled, &mut || false, None) {
            Ok(PrepareStep::Yielded) => unreachable!("unsliced task cannot yield"),
            Ok(PrepareStep::Complete(ready)) => Ok(ready),
            Ok(PrepareStep::Retired) => Ok(true),
            Err(error) => Err(error),
        };
        *ev = std::mem::replace(&mut task.ev, Evaluator::new());
        result
    }
}

/// A detached history has exactly one owner, including while queued between
/// CPU slices. No partial head or completion is published by a CPU yield.
pub(crate) struct PrepareTask {
    work: PrepareVoice,
    ev: Evaluator,
    static_content: bool,
    retime: RetimeCache,
    fixed: Option<crate::graph::StaticMap>,
    initialized: bool,
    first: i64,
    a: i64,
    last: i64,
    stage: PrepareStage,
    src: Vec<Vec<f32>>,
    out: Vec<Vec<f32>>,
    terminal: bool,
    #[cfg(test)]
    last_slice_boundaries: u8,
    #[cfg(test)]
    before_history_lock: Option<Box<dyn FnOnce() + Send>>,
}
#[derive(Clone, Copy)]
enum PrepareStage {
    Join,
    Lead,
    Head,
}
pub(crate) enum PrepareStep {
    Yielded,
    Complete(bool),
    /// A live edit handed this history to the producer. Its old request
    /// must neither render again nor update a prepared slot with Done.
    Retired,
}
impl PrepareTask {
    pub(crate) fn step(&mut self, cancelled: impl Fn() -> bool) -> Result<PrepareStep> {
        let boundaries = Cell::new(0u8);
        let result = self.step_with(
            &cancelled,
            &mut || {
                let n = boundaries.get().saturating_add(1);
                boundaries.set(n);
                n >= 2
            },
            Some(&boundaries),
        );
        #[cfg(test)]
        {
            self.last_slice_boundaries = boundaries.get();
        }
        result
    }

    fn finish(&mut self, ready: bool) {
        if ready && let Some(head) = &self.work.head {
            head.available.store(true, Ordering::Release);
        }
        // step_inner released the history lock before publishing finished.
        self.work.finished.store(true, Ordering::Release);
        self.terminal = true;
    }

    pub(crate) fn cancel(&mut self) {
        if !self.terminal {
            self.finish(true);
        }
    }

    fn step_with(
        &mut self,
        cancelled: &dyn Fn() -> bool,
        should_yield: &mut dyn FnMut() -> bool,
        boundaries: Option<&Cell<u8>>,
    ) -> Result<PrepareStep> {
        debug_assert!(!self.terminal, "terminal preparation was rescheduled");
        let result = self.step_inner(cancelled, should_yield, boundaries);
        match &result {
            Ok(PrepareStep::Yielded) => {}
            Ok(PrepareStep::Complete(ready)) => self.finish(*ready),
            // The producer owns both the history and its handoff state.
            // Retiring the task must not publish any old head or flags.
            Ok(PrepareStep::Retired) => self.terminal = true,
            Err(_) => self.finish(true),
        }
        result
    }

    fn initialize(&mut self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let work = &self.work;
        crate::realtime::checkpoint_phase("prepare-resolve");
        let comps = work.graph.resolve(&work.doc)?;
        let plan = &work.graph.voices[work.vi];
        let p = work.rate as f64;
        let (first, last) = voice_bounds(&work.reader, plan, p);
        let a = work.start.max(first);
        let b = (work.start + crate::ring::CHUNK as i64).min(last);
        self.a = a;
        self.first = first;
        self.last = last;
        if b <= a {
            self.last = a;
            self.initialized = true;
            return Ok(());
        }
        crate::realtime::checkpoint_phase("prepare-map");
        let fixed = self
            .static_content
            .then(|| crate::graph::StaticMap::new(plan, &comps, &mut self.ev, a as f64 / p))
            .transpose()?
            .flatten();
        // Queue the whole bounded deterministic prefix before consuming
        // its first block. Discovering later blocks only after earlier
        // reads complete serializes decode latency across the pre-roll.
        let source = &plan.source;
        let block = (p * pm_stretch::DEFAULT_BLOCK_SECONDS).ceil();
        let lo =
            (a.div_euclid(stretch::GRID) * stretch::GRID - stretch::preroll(work.rate)).max(first);
        let hi = (b + 2 * block as i64).min(last);
        crate::realtime::checkpoint_phase("prepare-prefetch");
        if let Some(fixed) = &fixed {
            // A constant map is monotone. Two exact endpoint evaluations
            // bound it without resolving thousands of control points or
            // requiring an entire long source to be decoded.
            if plan.static_point.is_some_and(|pt| pt.active) {
                let rate = source.sample_rate() as f64;
                let x0 = fixed.evaluate(plan, lo as f64 / p).source_time * rate;
                let x1 = fixed.evaluate(plan, hi as f64 / p).source_time * rate;
                if x0.is_finite() && x1.is_finite() {
                    let step = plan.constant_rate.unwrap().abs() * rate / p;
                    let taps = match &work.project_reader {
                        Reader::Copy { .. } => 1,
                        Reader::Grid { bank, .. } => bank.taps,
                        Reader::Free => kernel::free_taps(rate / p),
                    };
                    // The stretch reads through the linear project-rate
                    // reader. Its taps plus a rounding margin are enough;
                    // no speed-specific resampling bank needs building.
                    let pad = (2.0 * block * step.max(rate / p)).ceil() as i64
                        + (2 * kernel::TAPS + taps) as i64;
                    let from = (x0.min(x1).floor() as i64).saturating_sub(pad);
                    let to = (x0.max(x1).ceil() as i64).saturating_add(pad);
                    if !work.cache.is_ready(source, from, (to - from) as usize) {
                        work.cache.prefetch_span(source, from, (to - from) as usize);
                    }
                }
            }
        } else {
            let cached = source
                .length()
                .and_then(|n| usize::try_from(n).ok())
                .is_some_and(|n| work.cache.is_ready(source, 0, n));
            if !cached {
                let _timing = crate::profile::timer(crate::profile::Kind::Bounds);
                let span = CONTROL * READ_GROUP;
                for group in lo.div_euclid(span)..=(hi - 1).div_euclid(span) {
                    crate::realtime::checkpoint();
                    if cancelled() {
                        return Err(crate::error::AudioError::Device(
                            "superseded preparation".into(),
                        ));
                    }
                    for (lo, hi) in group_spans(
                        plan,
                        None,
                        &mut self.ev,
                        &mut self.retime,
                        &comps,
                        group,
                        p,
                        (first, last),
                        Some(block),
                    )? {
                        work.cache.prefetch_span(source, lo, (hi - lo) as usize);
                    }
                }
            }
        }

        crate::realtime::checkpoint_phase("prepare-scratch");
        self.fixed = fixed;
        self.out = vec![Vec::new(); plan.source.channels()];
        self.initialized = true;
        Ok(())
    }

    fn step_inner(
        &mut self,
        cancelled: &dyn Fn() -> bool,
        should_yield: &mut dyn FnMut() -> bool,
        boundaries: Option<&Cell<u8>>,
    ) -> Result<PrepareStep> {
        if cancelled() {
            return Err(crate::error::AudioError::Device(
                "superseded preparation".into(),
            ));
        }
        if self.work.finished.load(Ordering::Acquire) {
            return Ok(PrepareStep::Retired);
        }
        if !self.initialized {
            self.initialize(cancelled)?;
        }
        if self.last <= self.a {
            return Ok(PrepareStep::Complete(true));
        }
        let work = &self.work;
        let comps = work.graph.resolve(&work.doc)?;
        let plan = &work.graph.voices[work.vi];
        let p = work.rate as f64;
        let a = self.a;
        let fixed = &self.fixed;
        let ev = &mut self.ev;
        let retime = &mut self.retime;
        let mut map = |c: i64| -> Result<MapPoint> {
            if cancelled() {
                return Err(crate::error::AudioError::Device(
                    "superseded preparation".into(),
                ));
            }
            if let Some(fixed) = fixed {
                return Ok(fixed.point((c * CONTROL) as f64 / p, p));
            }
            let pt = plan.evaluate(ev, &comps, &comps[0], (c * CONTROL) as f64 / p, retime)?;
            Ok(MapPoint {
                x: pt.source_time * p,
                on: pt.active && pt.in_span,
            })
        };
        let src = &mut self.src;
        let mut read = |lo, len, out: &mut [Vec<f32>]| {
            if cancelled() {
                return Err(crate::error::AudioError::Device(
                    "superseded preparation".into(),
                ));
            }
            read_linear(
                &work.cache,
                &plan.source,
                work.rate,
                &work.project_reader,
                lo,
                len,
                out,
                src,
                false,
            )
        };
        // While finished is false only this task owns the history, even
        // between slices. A live edit can retire it and hand the original
        // phase to the producer; check that transfer under the same mutex.
        #[cfg(test)]
        if let Some(hook) = self.before_history_lock.take() {
            hook();
        }
        let mut run = work.run.lock().unwrap_or_else(|e| e.into_inner());
        // An edit can retire this task and advance its now-audible history
        // while the continuation waits for the mutex. Its generation bump
        // happens before the producer releases that mutex. Check again
        // before fresh() can discard the producer's live phase state.
        if cancelled() {
            return Err(crate::error::AudioError::Device(
                "superseded preparation".into(),
            ));
        }
        if work.finished.load(Ordering::Acquire) {
            return Ok(PrepareStep::Retired);
        }
        loop {
            let outcome = match self.stage {
                PrepareStage::Join => run.prepare_join_budgeted(
                    a,
                    work.target.max(self.first),
                    &mut map,
                    &mut read,
                    &mut self.out,
                    should_yield,
                )?,
                PrepareStage::Lead => {
                    let frames = lead_frames(work.rate).min((self.last - a) as usize);
                    if frames > crate::ring::CHUNK {
                        run.render_budgeted(
                            a,
                            frames,
                            &mut map,
                            &mut read,
                            &mut self.out,
                            should_yield,
                        )?
                    } else {
                        stretch::PrepareOutcome::Complete
                    }
                }
                PrepareStage::Head => {
                    let frames = head_frames(work.rate).min((self.last - a) as usize);
                    run.render_budgeted(
                        a,
                        frames,
                        &mut map,
                        &mut read,
                        &mut self.out,
                        should_yield,
                    )?
                }
            };
            match outcome {
                stretch::PrepareOutcome::Yielded => return Ok(PrepareStep::Yielded),
                stretch::PrepareOutcome::MissingInput => {
                    // Optional continuous lead has always tolerated a later
                    // decoder miss: its first chunk remains complete/ready.
                    return Ok(PrepareStep::Complete(matches!(
                        self.stage,
                        PrepareStage::Lead
                    )));
                }
                stretch::PrepareOutcome::Complete => {}
            }
            // `render_budgeted` calls the predicate after every nonterminal
            // fresh/produce boundary. Its final native call completes the
            // render before it can yield, so charge that boundary here. The
            // quota spans Join → Lead/Head instead of granting a hidden new
            // two-boundary allowance at each stage. A class-changing
            // boundary can contain more than one native FFI call.
            let quota_exhausted = boundaries.is_some_and(|count| {
                let n = count.get().saturating_add(1);
                count.set(n);
                n >= 2
            });
            match self.stage {
                PrepareStage::Join if work.target != work.start => {
                    return Ok(PrepareStep::Complete(true));
                }
                PrepareStage::Join if work.head.is_none() => self.stage = PrepareStage::Lead,
                PrepareStage::Join => self.stage = PrepareStage::Head,
                PrepareStage::Lead => return Ok(PrepareStep::Complete(true)),
                PrepareStage::Head => {
                    let head = work.head.as_ref().expect("head stage has a shared slot");
                    let mut shared = head.lock().unwrap_or_else(|e| e.into_inner());
                    if shared.is_none() {
                        *shared = Some(Arc::new(Head {
                            start: a,
                            ready_us: crate::clock::host_now_us(),
                            pcm: std::mem::take(&mut self.out),
                        }));
                        head.available.store(true, Ordering::Release);
                    }
                    return Ok(PrepareStep::Complete(true));
                }
            }
            if quota_exhausted {
                // The completed stage is finalized but neither Done nor a
                // partial head is visible. The same task resumes at its new
                // stage after a fair FIFO turn.
                return Ok(PrepareStep::Yielded);
            }
        }
    }
}
impl Drop for PrepareTask {
    fn drop(&mut self) {
        // Cancellation can discard a queued, already-claimed continuation.
        // Releasing it must not strand a dependent on an immutable head.
        self.cancel();
    }
}

/// Detached histories for one start. A history handed to the audible mix
/// leaves its entry at once ([`Prepared::release`]): the entry then holds
/// only histories no voice plays, so no later job or reuse (a loop
/// wrap) ever touches a live run.
struct Prepared {
    runs: Vec<Option<Arc<std::sync::Mutex<StretchRun>>>>,
    ready: Vec<bool>,
    heads: Vec<Option<SharedHead>>,
    /// Each voice's latest submitted job's completion (see [`PrepareVoice`]).
    finished: Vec<Arc<AtomicBool>>,
    /// The claim of each voice's latest submitted job.
    started: Vec<Arc<AtomicBool>>,
    critical: Vec<Arc<AtomicBool>>,
}
impl Prepared {
    fn new(voices: usize) -> Self {
        Self {
            runs: (0..voices).map(|_| None).collect(),
            ready: vec![false; voices],
            heads: vec![None; voices],
            finished: (0..voices)
                .map(|_| Arc::new(AtomicBool::new(false)))
                .collect(),
            critical: (0..voices)
                .map(|_| Arc::new(AtomicBool::new(false)))
                .collect(),
            started: (0..voices)
                .map(|_| Arc::new(AtomicBool::new(false)))
                .collect(),
        }
    }
    /// Voice `i`'s history now belongs to the audible mix. Its slot starts
    /// over: a later use of this start (the next loop wrap) gets a fresh
    /// history, never the advanced live one.
    fn release(&mut self, i: usize) {
        self.runs[i] = None;
        self.ready[i] = false;
        self.heads[i] = None;
        self.finished[i] = Arc::new(AtomicBool::new(false));
        self.started[i] = Arc::new(AtomicBool::new(false));
        self.critical[i] = Arc::new(AtomicBool::new(false));
    }
    fn output_ready(&self, i: usize) -> bool {
        self.ready[i]
            || self.heads[i]
                .as_ref()
                .is_some_and(|h| h.lock().unwrap_or_else(|e| e.into_inner()).is_some())
    }
}

impl Mixer {
    /// A mixer for `graph` (built from `doc`). `ev` evaluates the document
    /// (configure its expression engine before passing it).
    pub fn new(
        format: MixFormat,
        doc: Arc<Document>,
        graph: Arc<MixGraph>,
        ev: Evaluator,
        cache: Arc<SourceCache>,
    ) -> Mixer {
        let mut m = Mixer {
            master_tap: Tap::new(format.channels()),
            format,
            doc,
            graph: graph.clone(),
            ev,
            retime: RetimeCache::new(),
            cache,
            voices: vec![],
            taps: vec![],
            master_gain: graph.master as f32,
            gains: vec![],
            locals: vec![],
            positions: vec![],
            steps: vec![],
            src: vec![],
            src_valid: vec![],
            voice_dropped_frames: 0,
            voice_out: vec![],
            strips: vec![],
            kernel: vec![],
            points: vec![],
            outgoing: vec![],
            glide_len: 0,
            glide_acc: vec![],
            glide_tmp: vec![],
            glide_strips: vec![],
            reads: HashMap::new(),
            prepared: HashMap::new(),
            standard_content: false,
        };
        m.glide_len = (m.format.sample_rate as usize * RETUNE_GLIDE_MS as usize / 1000).max(1);
        m.set_graph(m.doc.clone(), graph);
        m
    }

    pub fn format(&self) -> &MixFormat {
        &self.format
    }

    /// Replaces the evaluator (an expression engine, hooks).
    pub fn set_evaluator(&mut self, ev: Evaluator) {
        self.standard_content = false;
        for (voice, plan) in self.voices.iter_mut().zip(&self.graph.voices).chain(
            self.outgoing
                .iter_mut()
                .flat_map(|g| g.voices.iter_mut().zip(&g.graph.voices)),
        ) {
            voice.fixed = None;
            voice.prefill = None;
            if voice.stretch.is_some() {
                voice.stretch = Some(Arc::new(std::sync::Mutex::new(StretchRun::new(
                    plan.source.channels(),
                    self.format.sample_rate,
                    plan.map_key,
                ))));
            }
        }
        for outgoing in &mut self.outgoing {
            outgoing.retime.clear();
        }
        self.prepared.clear();
        self.ev = ev;
        self.retime.clear();
        self.reads.clear();
    }

    pub(crate) fn standard_content(&mut self, enabled: bool) {
        self.standard_content = enabled;
        let comps = self.graph.resolve(&self.doc).ok();
        for (voice, plan) in self.voices.iter_mut().zip(&self.graph.voices) {
            voice.fixed = if enabled {
                comps.as_ref().and_then(|comps| {
                    crate::graph::StaticMap::new(plan, comps, &mut self.ev, plan.static_at)
                        .ok()
                        .flatten()
                })
            } else {
                None
            };
        }
    }

    pub fn cache(&self) -> &Arc<SourceCache> {
        &self.cache
    }

    pub fn graph(&self) -> &Arc<MixGraph> {
        &self.graph
    }

    pub fn document(&self) -> &Arc<Document> {
        &self.doc
    }

    /// Swaps in a new document revision and its graph (between renders),
    /// at once: what renders next is the new mix (an export, a seek, a
    /// paused edit). Ends any retune glide.
    pub fn set_graph(&mut self, doc: Arc<Document>, graph: Arc<MixGraph>) {
        self.outgoing.clear();
        self.install(doc, graph);
    }

    /// A live edit during playback: the renders that follow fade from the
    /// current mix (heard with `solo_before` as its monitor solo) into
    /// `doc` and `graph` over [`RETUNE_GLIDE_MS`], counted in rendered
    /// samples (so a loop wrap inside the glide continues it). The fade is
    /// linear, as today's `linearRampToValueAtTime` glide: a level change
    /// glides exactly as today's gain ramp does, and a structural change
    /// (a clip added, moved, muted) crossfades instead of clicking.
    pub fn glide_to(
        &mut self,
        doc: Arc<Document>,
        graph: Arc<MixGraph>,
        solo_before: Option<HashSet<String>>,
    ) {
        if self.outgoing.len() >= MAX_GLIDES {
            self.outgoing.remove(0);
        }
        self.outgoing.push(Outgoing {
            doc: self.doc.clone(),
            graph: self.graph.clone(),
            voices: std::mem::take(&mut self.voices),
            master_gain: self.master_gain,
            retime: std::mem::take(&mut self.retime),
            solo: solo_before,
            done: 0,
        });
        self.install(doc, graph);
    }

    /// Ends retune glides at once (a seek or a new run starts on the
    /// current mix).
    pub fn end_glides(&mut self) {
        self.outgoing.clear();
    }

    /// Forgets every stretch's starved request: the next render after a
    /// seek or a new run is a new request, which starts by the pre-roll
    /// rules (so it equals an offline render from there), never a retry
    /// that walks a run over the gap.
    pub fn abandon_retries(&mut self) {
        let outgoing = self.outgoing.iter().flat_map(|g| g.voices.iter());
        for v in self.voices.iter().chain(outgoing) {
            if let Some(run) = &v.stretch {
                run.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .abandon_retry();
            }
        }
    }

    /// Mark only unstretched voices whose first-chunk source blocks are
    /// already decoded. Vocoder histories are exclusively worker-prepared.
    /// A ready slot remains ready even if its source blocks are later evicted.
    pub(crate) fn mark_unstretched_ready(&mut self, start: i64) {
        let plain: Vec<usize> = self
            .voices
            .iter()
            .enumerate()
            .filter_map(|(vi, voice)| {
                (voice.stretch.is_none() && !self.prepared.get(&start).is_some_and(|p| p.ready[vi]))
                    .then_some(vi)
            })
            .collect();
        let mut decoded = Vec::with_capacity(plain.len());
        for vi in plain {
            let (spans, complete) =
                self.voice_spans(start, crate::ring::CHUNK, usize::MAX, Some(vi));
            decoded.push((
                vi,
                complete
                    && spans.iter().all(|(source, lo, hi)| {
                        self.cache.is_ready(source, *lo, (hi - lo) as usize)
                    }),
            ));
        }
        if !self.prepared.contains_key(&start) && self.prepared.len() >= 3 {
            self.prepared.clear();
        }
        let prepared = self
            .prepared
            .entry(start)
            .or_insert_with(|| Prepared::new(self.voices.len()));
        for (vi, ready) in decoded {
            if ready {
                prepared.ready[vi] = true;
            }
        }
    }

    /// Queue detached work for the nearest sounding voices first. Ready
    /// histories remain reusable; a job owns only its separate pending run.
    #[cfg(test)]
    pub(crate) fn preparation_work(&mut self, start: i64, target: i64) -> Vec<PrepareVoice> {
        self.preparation_work_except(start, target, |_| false)
    }

    /// [`Self::preparation_work`] for voices without a job in flight
    /// (`pending`). Each returned job gets a fresh claim.
    pub(crate) fn preparation_work_except(
        &mut self,
        start: i64,
        target: i64,
        pending: impl Fn(usize) -> bool,
    ) -> Vec<PrepareVoice> {
        let prepared = self
            .prepared
            .entry(start)
            .or_insert_with(|| Prepared::new(self.voices.len()));
        let mut work = Vec::new();
        for (vi, voice) in self.voices.iter().enumerate() {
            if prepared.ready[vi] || voice.stretch.is_none() || pending(vi) {
                continue;
            }
            let plan = &self.graph.voices[vi];
            if prepared.heads[vi].is_none()
                && let Some(fixed) = &voice.fixed
            {
                let peer = (0..self.voices.len()).find(|&i| {
                    i != vi
                        && self.graph.voices[i].source.id() == plan.source.id()
                        && self.voices[i]
                            .fixed
                            .as_ref()
                            .is_some_and(|f| fixed.same_signal(f))
                        && voice_bounds(&voice.reader, plan, self.format.sample_rate as f64)
                            == voice_bounds(
                                &self.voices[i].reader,
                                &self.graph.voices[i],
                                self.format.sample_rate as f64,
                            )
                });
                if let Some(peer) = peer {
                    prepared.heads[vi] = Some(
                        prepared.heads[peer]
                            .clone()
                            .unwrap_or_else(|| Arc::new(HeadSlot::new())),
                    );
                }
            }
            let run = prepared.runs[vi].get_or_insert_with(|| {
                Arc::new(std::sync::Mutex::new(StretchRun::new(
                    plan.source.channels(),
                    self.format.sample_rate,
                    plan.map_key,
                )))
            });
            let gate = prepared.heads[vi]
                .as_ref()
                .filter(|head| {
                    (0..vi).any(|i| {
                        !prepared.ready[i]
                            && prepared.heads[i]
                                .as_ref()
                                .is_some_and(|h| Arc::ptr_eq(h, head))
                    })
                })
                .map(|head| head.available.clone());
            // A fresh claim and completion per job: a superseded job that
            // is still exiting can never mark this one finished.
            prepared.finished[vi] = Arc::new(AtomicBool::new(false));
            prepared.started[vi] = Arc::new(AtomicBool::new(false));
            prepared.critical[vi] = Arc::new(AtomicBool::new(false));
            work.push(PrepareVoice {
                start,
                target,
                vi,
                doc: self.doc.clone(),
                graph: self.graph.clone(),
                cache: self.cache.clone(),
                rate: self.format.sample_rate,
                reader: voice.reader.clone(),
                project_reader: voice.project_reader.clone().unwrap(),
                run: run.clone(),
                head: prepared.heads[vi].clone(),
                gate,
                critical: prepared.critical[vi].clone(),
                started: prepared.started[vi].clone(),
                finished: prepared.finished[vi].clone(),
            });
        }
        work.sort_by_key(|w| {
            let (first, last) = voice_bounds(&w.reader, &w.graph.voices[w.vi], w.rate as f64);
            if start < first {
                first - start
            } else {
                (start - last).max(0)
            }
        });
        work
    }

    pub(crate) fn prepared_voice(&mut self, start: i64, vi: usize, ready: bool) {
        // Only a history still in its entry is recorded: an installed one
        // was released and its job's result belongs to no start.
        if let Some(p) = self.prepared.get_mut(&start) {
            p.ready[vi] = ready;
        }
    }

    pub(crate) fn preparation_failed(&mut self, start: i64, vi: usize) {
        // The ordinary audible render reports evaluation/decode errors. Never
        // leave a failed job hidden behind a permanent preparation mask.
        if let Some(p) = self.prepared.get_mut(&start) {
            p.ready[vi] = true;
            p.runs[vi] = None;
        }
        self.voices[vi].preparing = None;
    }

    pub(crate) fn preparation_matches(&self, start: i64, vi: usize, run_id: usize) -> bool {
        self.prepared
            .get(&start)
            .and_then(|p| p.runs.get(vi))
            .and_then(|r| r.as_ref())
            .is_some_and(|r| Arc::as_ptr(r) as usize == run_id)
    }

    pub(crate) fn prefill_ready_at(&self, start: i64) -> Option<u64> {
        let p = self.prepared.get(&start)?;
        let mut ready = 0;
        for (i, voice) in self.voices.iter().enumerate() {
            if voice.stretch.is_none() {
                continue;
            }
            let slot = p.heads[i].as_ref()?;
            let head = slot.lock().unwrap_or_else(|e| e.into_inner());
            ready = ready.max(head.as_ref()?.ready_us);
        }
        (ready != 0).then_some(ready)
    }

    pub(crate) fn stretches_ready(&self, start: i64) -> bool {
        self.voices.iter().enumerate().all(|(vi, voice)| {
            voice.stretch.is_none()
                || self
                    .prepared
                    .get(&start)
                    .is_some_and(|p| p.output_ready(vi))
        })
    }

    pub(crate) fn begin_prepared(&mut self, start: i64) -> bool {
        if self.stretches_ready(start) {
            self.use_prepared(start);
            return false;
        }
        // A3's per-voice bound: only a voice whose own first-chunk input is
        // still missing may join late. The global readiness test that led
        // here does not say which voice that is.
        let missing: Vec<bool> = (0..self.voices.len())
            .map(|i| {
                self.voices[i].stretch.is_some()
                    && !self
                        .voice_spans(start, crate::ring::CHUNK, usize::MAX, Some(i))
                        .0
                        .iter()
                        .all(|(source, lo, hi)| {
                            self.cache.is_ready(source, *lo, (hi - lo) as usize)
                        })
            })
            .collect();
        for (i, voice) in self.voices.iter_mut().enumerate() {
            if voice.stretch.is_none() {
                continue;
            }
            if let Some(p) = self.prepared.get_mut(&start)
                && p.ready[i]
            {
                if let Some(run) = p.runs[i].take() {
                    voice.stretch = Some(run);
                }
            } else if let Some(p) = self.prepared.get_mut(&start)
                && let Some(head) = p.heads[i]
                    .as_ref()
                    .and_then(|h| h.lock().unwrap_or_else(|e| e.into_inner()).clone())
                && let Some(run) = p.runs[i].take()
            {
                // Even at A3's decoder bound, a complete PCM head is a
                // ready voice. Never silence it for an unfinished CPU job.
                // The history leaves the entry with the voice: it is marked
                // served, so no job is scheduled on it and the join skips it.
                p.critical[i].store(true, Ordering::Release);
                voice.stretch = Some(run);
                voice.prefill = Some(Prefill {
                    head,
                    finished: p.finished[i].clone(),
                });
                p.ready[i] = true;
            } else if missing[i] {
                voice.preparing = Some(start);
            }
            // Otherwise this voice's own input is ready and only its CPU job
            // is pending. It stays on its live history, which the ordinary
            // render primes by the canonical pre-roll: complete from the
            // first sample, never omitted for another voice's decoder.
        }
        true
    }

    pub(crate) fn join_prepared(&mut self, start: i64, at: i64) -> bool {
        let Some(p) = self.prepared.get_mut(&start) else {
            return false;
        };
        for (i, voice) in self.voices.iter_mut().enumerate() {
            if voice.preparing != Some(start) || !p.ready[i] {
                continue;
            }
            if let Some(run) = &p.runs[i] {
                if run
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .can_continue_at(at)
                {
                    voice.stretch = p.runs[i].take();
                    voice.preparing = None;
                } else {
                    p.ready[i] = false;
                }
            }
        }
        let complete = self.voices.iter().all(|v| v.preparing.is_none());
        if complete {
            self.prepared.remove(&start);
        }
        complete
    }
    pub(crate) fn preparing_ids(&self) -> HashSet<String> {
        self.voices
            .iter()
            .zip(&self.graph.voices)
            .filter(|(v, _)| v.preparing.is_some())
            .map(|(_, p)| p.id.clone())
            .collect()
    }
    pub(crate) fn discard_prepared(&mut self, start: i64) {
        self.prepared.remove(&start);
    }

    pub(crate) fn invalidate_prepared(&mut self) {
        self.prepared.clear();
    }

    pub(crate) fn abandon_prefills(&mut self) {
        for (voice, plan) in self.voices.iter_mut().zip(&self.graph.voices) {
            if let Some(prefill) = voice.prefill.take()
                && !prefill.finished.load(Ordering::Acquire)
            {
                // A cancelled worker owns the old history until it exits.
                // A new epoch must not wait on its mutex.
                voice.stretch = Some(Arc::new(std::sync::Mutex::new(StretchRun::new(
                    plan.source.channels(),
                    self.format.sample_rate,
                    plan.map_key,
                ))));
            }
        }
    }

    pub(crate) fn clear_preparing(&mut self) {
        for v in &mut self.voices {
            v.preparing = None;
        }
    }

    /// Every start in `starts` holds a ready prepared history.
    pub(crate) fn prepared_ready(&self, starts: &[i64]) -> bool {
        starts.iter().all(|s| {
            self.prepared
                .get(s)
                .is_some_and(|p| (0..p.ready.len()).all(|i| p.output_ready(i)))
        })
    }

    pub(crate) fn retain_prepared(&mut self, starts: &[i64]) {
        self.prepared.retain(|start, _| starts.contains(start));
    }

    /// Install every ready voice at this start. Partial preparation still
    /// saves work on an immediate Play; unprepared voices use ordinary pre-roll.
    /// Other starts survive Play and are replenished after a loop consumes one.
    ///
    /// Every installed history (ready, or behind a complete head with its job
    /// still queued or running) is released from the entry: the audible mix
    /// owns it now. Histories not installed stay, still preparing this start
    /// for its next use, and released slots are prepared afresh.
    pub(crate) fn use_prepared(&mut self, start: i64) {
        if let Some(p) = self.prepared.get_mut(&start) {
            for (i, voice) in self.voices.iter_mut().enumerate() {
                if p.ready[i] && p.runs[i].is_some() {
                    voice.stretch = p.runs[i].take();
                    voice.prefill = None;
                    p.release(i);
                } else if let Some(head) = p.heads[i]
                    .as_ref()
                    .and_then(|h| h.lock().unwrap_or_else(|e| e.into_inner()).clone())
                    && let Some(run) = p.runs[i].take()
                {
                    p.critical[i].store(true, Ordering::Release);
                    voice.stretch = Some(run);
                    voice.prefill = Some(Prefill {
                        head,
                        finished: p.finished[i].clone(),
                    });
                    p.release(i);
                }
            }
        }
    }

    /// A complete immutable head can be queued before its independent
    /// history finishes. Never turn a CPU wait at its end into silence.
    pub(crate) fn awaiting_history(&self, at: i64, frames: usize) -> bool {
        self.voices.iter().zip(&self.graph.voices).any(|(v, plan)| {
            let (first, last) = voice_bounds(&v.reader, plan, self.format.sample_rate as f64);
            let a = at.max(first);
            let b = (at + frames as i64).min(last);
            b > a
                && v.prefill.as_ref().is_some_and(|p| {
                    !p.finished.load(Ordering::Acquire) && !p.head.covers(a, (b - a) as usize)
                })
        })
    }

    /// Whether a retune glide is in progress.
    pub fn is_gliding(&self) -> bool {
        !self.outgoing.is_empty()
    }

    fn install(&mut self, doc: Arc<Document>, graph: Arc<MixGraph>) {
        // Project updates cancel the detached jobs. Installed histories
        // belong to the audible mixes and were released from their entries
        // (`use_prepared`), so no new job is submitted on them. Keep their
        // immutable prefix and resume from its original phase if the
        // cancelled history was not yet complete.
        for voice in self
            .voices
            .iter()
            .chain(self.outgoing.iter().flat_map(|g| &g.voices))
        {
            if let Some(prefill) = &voice.prefill {
                prefill.finished.store(true, Ordering::Release);
            }
        }
        // A level/fade edit does not change the signal entering a vocoder.
        // Dynamic plans can depend on other properties: invalidate those.
        let old = &self.graph.voices;
        for prepared in self.prepared.values_mut() {
            let mut runs = Vec::with_capacity(graph.voices.len());
            let mut ready = Vec::with_capacity(graph.voices.len());
            let mut heads = Vec::with_capacity(graph.voices.len());
            let mut finished = Vec::with_capacity(graph.voices.len());
            let mut started = Vec::with_capacity(graph.voices.len());
            let mut critical = Vec::with_capacity(graph.voices.len());
            for plan in &graph.voices {
                let previous = old.iter().position(|p| {
                    !p.animated
                        && !plan.animated
                        && p.constant_rate.is_some()
                        && plan.constant_rate.is_some()
                        && p.id == plan.id
                        && p.map_key == plan.map_key
                });
                runs.push(previous.and_then(|i| prepared.runs[i].take()));
                ready.push(previous.is_some_and(|i| prepared.ready[i]));
                heads.push(previous.and_then(|i| prepared.heads[i].clone()));
                finished.push(
                    previous
                        .map(|i| prepared.finished[i].clone())
                        .unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
                );
                started.push(
                    previous
                        .map(|i| prepared.started[i].clone())
                        .unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
                );
                critical.push(
                    previous
                        .map(|i| prepared.critical[i].clone())
                        .unwrap_or_else(|| Arc::new(AtomicBool::new(false))),
                );
            }
            prepared.runs = runs;
            prepared.ready = ready;
            prepared.heads = heads;
            prepared.finished = finished;
            prepared.started = started;
            prepared.critical = critical;
        }
        let p = self.format.sample_rate;
        // Stretches in progress carry over to voices whose map did not
        // change (a fader drag during playback must not restart them).
        let mut runs: HashMap<(String, u64), Arc<std::sync::Mutex<StretchRun>>> = HashMap::new();
        let mut prefills: HashMap<(String, u64), Prefill> = HashMap::new();
        let previous = self.graph.voices.iter().zip(&self.voices).chain(
            self.outgoing
                .last()
                .map(|g| g.graph.voices.iter().zip(&g.voices))
                .into_iter()
                .flatten(),
        );
        for (plan, voice) in previous {
            if let Some(prefill) = &voice.prefill {
                prefills
                    .entry((plan.id.clone(), plan.map_key))
                    .or_insert_with(|| prefill.clone());
            }
            if let Some(run) = &voice.stretch {
                runs.entry((plan.id.clone(), plan.map_key))
                    .or_insert_with(|| run.clone());
            }
        }
        self.voices = graph
            .voices
            .iter()
            .map(|v| {
                let stretched = stretches(v, p);
                Voice {
                    reader: reader_for(v, p),
                    matrix: layout::matrix(&v.source.info.speakers, &self.format.speakers),
                    strip: graph.strips.iter().position(|s| *s == v.strip).unwrap_or(0),
                    stretch: stretched.then(|| {
                        runs.get(&(v.id.clone(), v.map_key))
                            .cloned()
                            .unwrap_or_else(|| {
                                Arc::new(std::sync::Mutex::new(StretchRun::new(
                                    v.source.channels(),
                                    p,
                                    v.map_key,
                                )))
                            })
                    }),
                    project_reader: stretched.then(|| linear_reader(v.source.sample_rate(), p)),
                    preparing: None,
                    fixed: None,
                    prefill: prefills.get(&(v.id.clone(), v.map_key)).cloned(),
                }
            })
            .collect();
        if self.taps.len() != graph.strips.len() {
            self.taps = (0..graph.strips.len())
                .map(|_| Tap::new(self.format.channels()))
                .collect();
        }
        self.master_gain = graph.master as f32;
        self.retime.clear();
        self.reads.clear();
        self.doc = doc;
        self.graph = graph;
        self.standard_content(self.standard_content);
    }

    /// The latest levels (after a render with `meters`).
    pub fn meters(&self, position: i64) -> Meters {
        Meters {
            position,
            strips: self
                .graph
                .strips
                .iter()
                .zip(&self.taps)
                .map(|(id, tap)| (id.clone(), tap.levels()))
                .collect(),
            master: self.master_tap.levels(),
        }
    }

    pub fn reset_meters(&mut self) {
        for t in &mut self.taps {
            t.clear();
        }
        self.master_tap.clear();
    }

    /// Mixes project samples `[start, start + frames)` into `out` (one
    /// vector per bus channel, resized to `frames`).
    pub fn render(
        &mut self,
        start: i64,
        frames: usize,
        out: &mut [Vec<f32>],
        options: &RenderOptions,
    ) -> Result<RenderReport> {
        let mut report =
            self.render_current(start, frames, out, options, options.monitor_solo.as_ref())?;
        if !self.outgoing.is_empty() && frames > 0 {
            // The outgoing mixes render into spare strips: the current
            // mix's strips feed its meters below.
            std::mem::swap(&mut self.strips, &mut self.glide_strips);
            self.render_glides(start, frames, out, options, &mut report);
            std::mem::swap(&mut self.strips, &mut self.glide_strips);
        }
        // A starved render is retried (playback) or not heard as rendered:
        // the meters take only complete renders, once each.
        if options.meters && report.waiting_voices.is_empty() {
            for (tap, strip) in self.taps.iter_mut().zip(&self.strips) {
                tap.push(strip, frames);
            }
            self.master_tap.push(out, frames);
        }
        Ok(report)
    }

    /// Blends the outgoing mixes of the retune glides under `out` (the
    /// current mix): oldest first, each faded into the next.
    fn render_glides(
        &mut self,
        start: i64,
        frames: usize,
        out: &mut [Vec<f32>],
        options: &RenderOptions,
        report: &mut RenderReport,
    ) {
        let channels = self.format.channels();
        let len = self.glide_len as f32;
        let mut acc = std::mem::take(&mut self.glide_acc);
        let mut tmp = std::mem::take(&mut self.glide_tmp);
        acc.resize_with(channels, Vec::new);
        tmp.resize_with(channels, Vec::new);
        let mut outgoing = std::mem::take(&mut self.outgoing);
        let n = outgoing.len();
        for j in 0..n {
            let target = if j == 0 { &mut acc } else { &mut tmp };
            self.swap_state(&mut outgoing[j]);
            let solo = outgoing[j].solo.clone();
            // An outgoing mix that fails renders silence under the fade.
            match self.render_current(start, frames, target, options, solo.as_ref()) {
                Ok(outgoing_report) => {
                    report.starved += outgoing_report.starved;
                    for id in outgoing_report.starved_voices {
                        if !report.starved_voices.contains(&id) {
                            report.starved_voices.push(id);
                        }
                    }
                    for id in outgoing_report.waiting_voices {
                        if !report.waiting_voices.contains(&id) {
                            report.waiting_voices.push(id);
                        }
                    }
                    for id in outgoing_report.heard_whole {
                        if !report.heard_whole.contains(&id) {
                            report.heard_whole.push(id);
                        }
                    }
                    for (id, frames) in outgoing_report.dropped_by_voice {
                        match report.dropped_by_voice.iter_mut().find(|(v, _)| *v == id) {
                            Some((_, counted)) if *counted >= frames => {}
                            Some((_, counted)) => {
                                report.dropped_voice_frames += frames - *counted;
                                *counted = frames;
                            }
                            None => {
                                report.dropped_voice_frames += frames;
                                report.dropped_by_voice.push((id, frames));
                            }
                        }
                    }
                    report.errors.extend(outgoing_report.errors);
                }
                Err(e) => {
                    report.errors.push(("retune".into(), e.to_string()));
                    for c in target.iter_mut() {
                        c.clear();
                        c.resize(frames, 0.0);
                    }
                }
            }
            self.swap_state(&mut outgoing[j]);
            if j > 0 {
                let done = outgoing[j - 1].done;
                blend(&mut acc, &tmp, done, len);
            }
        }
        // The newest glide fades `acc` into the current mix: out = acc·(1 − w) + out·w.
        let done = outgoing[n - 1].done;
        for (o, a) in out.iter_mut().zip(&acc) {
            for (i, (v, x)) in o.iter_mut().zip(a).enumerate() {
                let w = ((done + i) as f32 / len).min(1.0);
                *v = *x + (*v - *x) * w;
            }
        }
        for g in &mut outgoing {
            // Playback retries a segment whose source was missing. Its
            // retune envelope must stay at the same rendered position.
            if report.waiting_voices.is_empty() {
                g.done += frames;
            }
        }
        // A finished glide leaves the mixes before it unheard.
        if let Some(last) = outgoing.iter().rposition(|g| g.done >= self.glide_len) {
            outgoing.drain(..=last);
        }
        self.outgoing = outgoing;
        self.glide_acc = acc;
        self.glide_tmp = tmp;
    }

    fn swap_state(&mut self, g: &mut Outgoing) {
        std::mem::swap(&mut self.doc, &mut g.doc);
        std::mem::swap(&mut self.graph, &mut g.graph);
        std::mem::swap(&mut self.voices, &mut g.voices);
        std::mem::swap(&mut self.master_gain, &mut g.master_gain);
        std::mem::swap(&mut self.retime, &mut g.retime);
    }

    /// Mixes the current state (no glides, no meters).
    fn render_current(
        &mut self,
        start: i64,
        frames: usize,
        out: &mut [Vec<f32>],
        options: &RenderOptions,
        monitor_solo: Option<&HashSet<String>>,
    ) -> Result<RenderReport> {
        let channels = self.format.channels();
        assert_eq!(out.len(), channels, "one output vector per bus channel");
        for c in out.iter_mut() {
            c.clear();
            c.resize(frames, 0.0);
        }
        let mut report = RenderReport::default();
        let strips = self.graph.strips.len();
        self.strips.resize_with(strips, Vec::new);
        for s in &mut self.strips {
            s.resize_with(channels, Vec::new);
            for c in s.iter_mut() {
                c.clear();
                c.resize(frames, 0.0);
            }
        }
        if frames == 0 {
            return Ok(report);
        }
        let doc = self.doc.clone();
        let graph = self.graph.clone();
        let comps = graph.resolve(&doc)?;
        let open = comps[0].clone();
        let solo = monitor_solo.filter(|s| !s.is_empty());
        for (vi, plan) in graph.voices.iter().enumerate() {
            crate::realtime::checkpoint();
            self.voice_dropped_frames = 0;
            let mut voice_options;
            let options = if options.drop_voices.contains(&plan.id) && !options.wait {
                voice_options = options.clone();
                voice_options.drop_starved = true;
                &voice_options
            } else {
                options
            };
            match self.render_voice(vi, plan, &comps, &open, start, frames, options) {
                Ok(Some(starved)) => {
                    report.voices += 1;
                    report.starved += starved as usize;
                    if starved {
                        if !options.drop_starved && !report.waiting_voices.contains(&plan.id) {
                            report.waiting_voices.push(plan.id.clone());
                        }
                        if !report.starved_voices.contains(&plan.id) {
                            report.starved_voices.push(plan.id.clone());
                        }
                        if self.voice_dropped_frames > 0 {
                            report.dropped_voice_frames += self.voice_dropped_frames;
                            report
                                .dropped_by_voice
                                .push((plan.id.clone(), self.voice_dropped_frames));
                        }
                    } else if solo.is_none_or(|s| s.contains(&plan.strip))
                        && !report.heard_whole.contains(&plan.id)
                    {
                        report.heard_whole.push(plan.id.clone());
                    }
                }
                Ok(None) => {}
                Err(e) => report.errors.push((plan.id.clone(), e.to_string())),
            }
        }
        for (si, strip) in self.strips.iter().enumerate() {
            if solo.is_some_and(|s| !s.contains(&graph.strips[si])) {
                continue;
            }
            for (o, c) in out.iter_mut().zip(strip) {
                for (a, b) in o.iter_mut().zip(c) {
                    *a += *b;
                }
            }
        }
        let master = self.master_gain;
        if master != 1.0 {
            for o in out.iter_mut() {
                for v in o.iter_mut() {
                    *v *= master;
                }
            }
        }
        Ok(report)
    }

    /// Mixes one voice into its strip. `Ok(None)` when it is silent in the
    /// range, else whether it was starved.
    #[allow(clippy::too_many_arguments)]
    fn render_voice(
        &mut self,
        vi: usize,
        plan: &VoicePlan,
        comps: &[pm_eval::Comp<'_>],
        open: &pm_eval::Comp<'_>,
        start: i64,
        frames: usize,
        options: &RenderOptions,
    ) -> Result<Option<bool>> {
        let p = self.format.sample_rate as f64;
        let end = start + frames as i64;
        let (first, last) = voice_bounds(&self.voices[vi].reader, plan, p);
        let (a, b) = (start.max(first), end.min(last));
        if b <= a {
            return Ok(None);
        }
        let n = (b - a) as usize;
        if self.voices[vi].preparing.is_some() {
            if options.drop_starved {
                self.voice_dropped_frames += n as u64;
            }
            return Ok(Some(true));
        }
        let offset = (a - start) as usize;
        self.gains.clear();
        self.gains.resize(n, 0.0);
        let linear = plan.offset.is_some();
        let source_rate = plan.source.sample_rate() as f64;
        if let (false, Some(pt)) = (plan.animated, plan.static_point.filter(|_| linear)) {
            // Static and linear: the local time is the master time shifted.
            if !pt.active || pt.gain == 0.0 {
                return Ok(None);
            }
            let local0 = plan.static_at - pt.local;
            for (i, g) in self.gains.iter_mut().enumerate() {
                let t = (a + i as i64) as f64 / p;
                *g = (pt.gain * pt.fade(t - local0)) as f32;
            }
        } else {
            // Control points at multiples of CONTROL around [a, b].
            let c0 = a.div_euclid(CONTROL) * CONTROL;
            let c1 = (b + CONTROL - 1).div_euclid(CONTROL) * CONTROL;
            self.points.clear();
            let mut c = c0;
            while c <= c1 {
                let t = c as f64 / p;
                let pt = if let Some(fixed) = &self.voices[vi].fixed {
                    fixed.evaluate(plan, t)
                } else {
                    plan.evaluate(&mut self.ev, comps, open, t, &mut self.retime)?
                };
                self.points.push(pt);
                c += CONTROL;
            }
            let on = |pt: &Point| pt.active && (linear || pt.in_span);
            if self.points.iter().all(|pt| !on(pt) || pt.gain == 0.0) {
                return Ok(None);
            }
            self.locals.clear();
            self.positions.clear();
            self.steps.clear();
            for i in 0..n {
                let s = a + i as i64;
                let k = ((s - c0) / CONTROL) as usize;
                let x = ((s - c0) % CONTROL) as f64 / CONTROL as f64;
                let (l, r) = (
                    &self.points[k],
                    &self.points[(k + 1).min(self.points.len() - 1)],
                );
                let gl = if on(l) { l.gain } else { 0.0 };
                let gr = if on(r) { r.gain } else { 0.0 };
                let gain = gl + (gr - gl) * x;
                let local = l.local + (r.local - l.local) * x;
                // Fade parameters hold from the left control point.
                let fade = if linear {
                    let t = s as f64 / p;
                    let local0 = c0 as f64 / p - self.points[0].local;
                    l.fade(t - local0)
                } else {
                    l.fade(local)
                };
                self.gains[i] = (gain * fade) as f32;
                if !linear {
                    let st = l.source_time + (r.source_time - l.source_time) * x;
                    self.positions.push(st * source_rate);
                    // The read rate of the control interval (not of this
                    // render's samples), so the filter is the same whatever
                    // block the sample falls in.
                    self.steps
                        .push((r.source_time - l.source_time).abs() * source_rate / CONTROL as f64);
                }
                self.locals.push(local);
            }
        }
        if let (Some(run), Some(reader), false) = (
            self.voices[vi].stretch.clone(),
            &self.voices[vi].project_reader,
            options.no_stretch,
        ) {
            // A stretch advances whatever the level (a run's state must not
            // depend on where gains happen to be zero).
            return self
                .render_stretched(
                    vi,
                    plan,
                    comps,
                    open,
                    a,
                    n,
                    offset,
                    &run,
                    reader.clone(),
                    options,
                )
                .map(Some);
        }
        if self.gains.iter().all(|&g| g == 0.0) {
            return Ok(None);
        }
        let voice = &self.voices[vi];
        if let (Reader::Free, Some(o)) = (&voice.reader, plan.offset) {
            // A linear map at a ratio too fine for an exact-phase bank.
            self.positions.clear();
            self.positions
                .extend((0..n).map(|i| ((a + i as i64) as f64 / p - o) * source_rate));
        }
        let source = plan.source.clone();
        let sc = source.channels();
        self.src.resize_with(sc, Vec::new);
        self.voice_out.resize_with(sc, Vec::new);
        for c in self.voice_out.iter_mut() {
            c.clear();
            c.resize(n, 0.0);
        }
        let mut complete = true;
        match &voice.reader {
            Reader::Copy { shift } => {
                for c in self.voice_out.iter_mut() {
                    c.resize(n, 0.0);
                }
                let mut views: Vec<&mut [f32]> =
                    self.voice_out.iter_mut().map(|c| &mut c[..n]).collect();
                self.src_valid.resize(n, true);
                complete &= self.cache.read_valid(
                    &source,
                    a - shift,
                    n,
                    &mut views,
                    options.wait,
                    options
                        .drop_starved
                        .then_some(self.src_valid.as_mut_slice()),
                )?;
                if options.drop_starved {
                    // Like the other readers, only frames that would be
                    // heard count as dropped.
                    self.voice_dropped_frames += self.src_valid[..n]
                        .iter()
                        .zip(&self.gains[..n])
                        .filter(|&(&valid, &gain)| !valid && gain != 0.0)
                        .count() as u64;
                }
            }
            Reader::Grid {
                bank,
                step,
                origin,
                phases,
            } => {
                let unit = |s: i64| *origin + s as i128 * *step;
                let lo = unit(a).div_euclid(*phases) as i64 - bank.before() as i64;
                let hi =
                    unit(b - 1).div_euclid(*phases) as i64 + (bank.taps - bank.before()) as i64;
                let span = (hi - lo) as usize;
                for c in self.src.iter_mut() {
                    c.clear();
                    c.resize(span, 0.0);
                }
                let mut views: Vec<&mut [f32]> =
                    self.src.iter_mut().map(|c| &mut c[..span]).collect();
                self.src_valid.resize(span, true);
                complete &= self.cache.read_valid(
                    &source,
                    lo,
                    span,
                    &mut views,
                    options.wait,
                    options
                        .drop_starved
                        .then_some(self.src_valid.as_mut_slice()),
                )?;
                for i in 0..n {
                    if self.gains[i] == 0.0 {
                        continue;
                    }
                    let u = unit(a + i as i64);
                    let (si, ph) = (u.div_euclid(*phases) as i64, u.rem_euclid(*phases) as usize);
                    if !complete
                        && options.drop_starved
                        && !self.src_valid[(si - bank.before() as i64 - lo) as usize
                            ..(si - bank.before() as i64 - lo) as usize + bank.taps]
                            .iter()
                            .all(|&v| v)
                    {
                        self.voice_dropped_frames += 1;
                        continue;
                    }
                    for c in 0..sc {
                        self.voice_out[c][i] = bank.grid(&self.src[c], lo, si, ph);
                    }
                }
            }
            Reader::Free => {
                // Per control interval, so a jump in a remap never reads a
                // long span.
                let mut i = 0;
                while i < n {
                    let s = a + i as i64;
                    let run = ((CONTROL - s.rem_euclid(CONTROL)) as usize).min(n - i);
                    let seg = &self.positions[i..i + run];
                    let silent = self.gains[i..i + run].iter().all(|&g| g == 0.0);
                    if silent {
                        i += run;
                        continue;
                    }
                    let step = self.steps[i..i + run].iter().fold(1.0f64, |m, &s| m.max(s));
                    let bank = kernel::free_bank(step);
                    let (mut pmin, mut pmax) = (f64::INFINITY, f64::NEG_INFINITY);
                    for &pos in seg {
                        pmin = pmin.min(pos);
                        pmax = pmax.max(pos);
                    }
                    let lo = pmin.floor() as i64 - bank.before() as i64;
                    let hi = pmax.floor() as i64 + (bank.taps - bank.before()) as i64;
                    let span = (hi - lo) as usize;
                    for c in self.src.iter_mut() {
                        c.clear();
                        c.resize(span, 0.0);
                    }
                    let mut views: Vec<&mut [f32]> =
                        self.src.iter_mut().map(|c| &mut c[..span]).collect();
                    self.src_valid.resize(span, true);
                    complete &= self.cache.read_valid(
                        &source,
                        lo,
                        span,
                        &mut views,
                        options.wait,
                        options
                            .drop_starved
                            .then_some(self.src_valid.as_mut_slice()),
                    )?;
                    self.kernel.resize(bank.taps, 0.0);
                    for (j, &pos) in seg.iter().enumerate() {
                        if self.gains[i + j] == 0.0 {
                            continue;
                        }
                        let fl = pos.floor();
                        bank.free_kernel(pos - fl, &mut self.kernel);
                        let first = fl as i64 - bank.before() as i64;
                        if !complete
                            && options.drop_starved
                            && !self.src_valid
                                [(first - lo) as usize..(first - lo) as usize + bank.taps]
                                .iter()
                                .all(|&v| v)
                        {
                            self.voice_dropped_frames += 1;
                            continue;
                        }
                        let at = (first - lo) as usize;
                        for c in 0..sc {
                            self.voice_out[c][i + j] =
                                kernel::dot(&self.src[c][at..at + bank.taps], &self.kernel);
                        }
                    }
                    i += run;
                }
            }
        }
        // Fold into the strip with the per-sample gain.
        let strip = &mut self.strips[voice.strip];
        for (bus, row) in strip.iter_mut().zip(&voice.matrix) {
            for (c, &m) in row.iter().enumerate() {
                if m == 0.0 {
                    continue;
                }
                let src = &self.voice_out[c];
                let dst = &mut bus[offset..offset + n];
                for i in 0..n {
                    dst[i] += src[i] * (m * self.gains[i]);
                }
            }
        }
        Ok(Some(!complete))
    }

    /// Mixes a stretched voice (see [`crate::stretch`]) into its strip,
    /// with the gains [`Self::render_voice`] computed. Returns whether it
    /// was starved.
    #[allow(clippy::too_many_arguments)]
    fn render_stretched(
        &mut self,
        vi: usize,
        plan: &VoicePlan,
        comps: &[pm_eval::Comp<'_>],
        open: &pm_eval::Comp<'_>,
        a: i64,
        n: usize,
        offset: usize,
        run: &Arc<std::sync::Mutex<StretchRun>>,
        reader: Reader,
        options: &RenderOptions,
    ) -> Result<bool> {
        let p = self.format.sample_rate as f64;
        let head = self.voices[vi]
            .prefill
            .as_ref()
            .map(|p| p.head.clone())
            .filter(|h| h.covers(a, n));
        let complete = if let Some(head) = head {
            self.voice_out.resize_with(plan.source.channels(), Vec::new);
            let from = (a - head.start) as usize;
            for (out, input) in self.voice_out.iter_mut().zip(&head.pcm) {
                out.clear();
                out.extend_from_slice(&input[from..from + n]);
            }
            true
        } else {
            let mut run = run.lock().unwrap_or_else(|e| e.into_inner());
            // The control points the gains used, as map points.
            if !self.points.is_empty() {
                let c0 = a.div_euclid(CONTROL);
                run.offer(
                    c0,
                    self.points.iter().map(|pt| MapPoint {
                        x: pt.source_time * p,
                        on: pt.active && pt.in_span,
                    }),
                );
            }
            let source = plan.source.clone();
            let cache = self.cache.clone();
            let wait = options.wait;
            let project = self.format.sample_rate;
            let (ev, retime) = (&mut self.ev, &mut self.retime);
            let mut map = |c: i64| -> Result<MapPoint> {
                let pt = plan.evaluate(ev, comps, open, (c * CONTROL) as f64 / p, retime)?;
                Ok(MapPoint {
                    x: pt.source_time * p,
                    on: pt.active && pt.in_span,
                })
            };
            self.voice_out.resize_with(source.channels(), Vec::new);
            let mut src = std::mem::take(&mut self.src);
            let mut read = |lo: i64, len: usize, out: &mut [Vec<f32>]| -> Result<bool> {
                read_linear(
                    &cache, &source, project, &reader, lo, len, out, &mut src, wait,
                )
            };
            let complete = if let Some(prefill) = &self.voices[vi].prefill
                && !run.can_continue_at(a)
            {
                if run.prepare_join(
                    prefill.head.start,
                    a,
                    &mut map,
                    &mut read,
                    &mut self.voice_out,
                )? {
                    run.render(a, n, &mut map, &mut read, &mut self.voice_out)
                } else {
                    Ok(false)
                }
            } else {
                run.render(a, n, &mut map, &mut read, &mut self.voice_out)
            };
            self.src = src;
            let complete = complete?;
            if !complete && options.drop_starved {
                self.voice_dropped_frames += n as u64;
                // Keep valid vocoder history through short dropped spans. The
                // run bounds catch-up work itself; missing PCM never enters DSP.
                run.commit_drop(a);
            }
            drop(run);
            complete
        };
        let voice = &self.voices[vi];
        let strip = &mut self.strips[voice.strip];
        for (bus, row) in strip.iter_mut().zip(&voice.matrix) {
            for (c, &m) in row.iter().enumerate() {
                if m == 0.0 {
                    continue;
                }
                let src = &self.voice_out[c];
                let dst = &mut bus[offset..offset + n];
                for i in 0..n {
                    dst[i] += src[i] * (m * self.gains[i]);
                }
            }
        }
        Ok(!complete)
    }

    /// The source sample spans `[lo, hi)` a render of project samples
    /// `[start, start + frames)` reads: linear voices first, then
    /// time-mapped voices in time order.
    ///
    /// Linear voices map the range directly. Time-mapped voices (speed,
    /// remaps, animated trims) are bounded per control interval from the
    /// same control points [`Self::render`] evaluates, padded by the filter
    /// the render reads with there, so the spans hold every sample the
    /// render touches. Those bounds are kept per group of [`READ_GROUP`]
    /// control points until the graph changes, so the look-ahead and the
    /// pre-roll poll evaluate each point once.
    pub fn source_spans(&mut self, start: i64, frames: usize) -> Vec<(Arc<Source>, i64, i64)> {
        self.source_spans_within(start, frames, usize::MAX).0
    }

    /// [`Self::source_spans`], evaluating at most `budget` groups not yet
    /// bounded (nearest first); also whether every group was bounded.
    pub fn source_spans_within(
        &mut self,
        start: i64,
        frames: usize,
        budget: usize,
    ) -> (Vec<(Arc<Source>, i64, i64)>, bool) {
        self.voice_spans(start, frames, budget, None)
    }

    /// [`Self::source_spans_within`], for one voice (`only`) or all of them.
    fn voice_spans(
        &mut self,
        start: i64,
        frames: usize,
        budget: usize,
        only: Option<usize>,
    ) -> (Vec<(Arc<Source>, i64, i64)>, bool) {
        let _timing = crate::profile::timer(crate::profile::Kind::Bounds);
        let p = self.format.sample_rate as f64;
        let graph = self.graph.clone();
        let doc = self.doc.clone();
        let end = start + frames as i64;
        let mut out = Vec::new();
        // Time-mapped voices: (index, first and last sample heard here).
        let mut mapped: Vec<(usize, i64, i64)> = Vec::new();
        for (vi, plan) in graph.voices.iter().enumerate() {
            crate::realtime::checkpoint();
            if only.is_some_and(|o| o != vi) {
                continue;
            }
            // A primed first chunk already contains its input, including the
            // deterministic pre-roll. Do not evaluate its bounds a second time.
            if frames <= crate::ring::CHUNK && self.voices[vi].preparing.is_some() {
                continue;
            }
            if frames <= crate::ring::CHUNK
                && self.voices[vi].stretch.is_some()
                && self
                    .prepared
                    .get(&start)
                    .is_some_and(|p| p.output_ready(vi))
            {
                continue;
            }
            let (first, last) = voice_bounds(&self.voices[vi].reader, plan, p);
            let (a, b) = (start.max(first), end.min(last));
            if b <= a {
                continue;
            }
            let Some(o) = plan.offset else {
                mapped.push((vi, first, last));
                continue;
            };
            let rate = plan.source.sample_rate() as f64;
            let pad = match &self.voices[vi].reader {
                Reader::Copy { .. } => 1,
                Reader::Grid { bank, .. } => bank.taps as i64,
                Reader::Free => kernel::free_taps(rate / p) as i64,
            };
            let s0 = ((a as f64 / p - o) * rate).floor() as i64 - pad;
            let s1 = ((b as f64 / p - o) * rate).ceil() as i64 + pad;
            out.push((plan.source.clone(), s0, s1));
        }
        if mapped.is_empty() || frames == 0 {
            return (out, true);
        }
        let Ok(comps) = graph.resolve(&doc) else {
            return (out, true);
        };
        let span = CONTROL * READ_GROUP;
        let mut left = budget;
        let mut complete = true;
        let block_p = (p * pm_stretch::DEFAULT_BLOCK_SECONDS).ceil();
        // A stretched voice reads from its fresh start (up to a grid step
        // and the pre-roll before `start`) and one output latency beyond
        // `end`.
        let pre = stretch::GRID + stretch::preroll(self.format.sample_rate);
        let groups = |from: i64, to: i64| from.div_euclid(span)..=(to - 1).div_euclid(span);
        let all = groups(start - pre, end + 2 * block_p as i64);
        for g in all {
            let (g0, g1) = (g * span, (g + 1) * span);
            for &(vi, first, last) in &mapped {
                crate::realtime::checkpoint();
                if g1 <= first || g0 >= last {
                    continue;
                }
                let stretched = self.voices[vi].stretch.is_some();
                let (lo, hi) = if stretched {
                    (
                        start.div_euclid(stretch::GRID) * stretch::GRID
                            - stretch::preroll(self.format.sample_rate),
                        end + 2 * block_p as i64,
                    )
                } else {
                    (start, end)
                };
                if g1 <= lo || g0 >= hi {
                    continue;
                }
                let plan = &graph.voices[vi];
                let spans = match self.reads.get(&(vi, g)) {
                    Some(s) => s.clone(),
                    None if left == 0 => {
                        complete = false;
                        continue;
                    }
                    None => {
                        left -= 1;
                        if self.reads.len() >= MAX_READ_GROUPS {
                            self.reads.clear();
                        }
                        // A voice that fails to evaluate reads nothing (the
                        // render reports the error).
                        let s: Spans = group_spans(
                            plan,
                            self.voices[vi].fixed.as_ref(),
                            &mut self.ev,
                            &mut self.retime,
                            &comps,
                            g,
                            p,
                            (first, last),
                            stretched.then_some(block_p),
                        )
                        .unwrap_or_default()
                        .into();
                        self.reads.insert((vi, g), s.clone());
                        s
                    }
                };
                out.extend(spans.iter().map(|&(lo, hi)| (plan.source.clone(), lo, hi)));
            }
        }
        (out, complete)
    }

    /// Queues the source blocks a playback of `[start, start + frames)`
    /// will need, for the decode workers.
    pub fn prefetch(&mut self, start: i64, frames: usize) {
        self.prefetch_within(start, frames, usize::MAX);
    }

    /// [`Self::prefetch`] bounding at most `budget` groups of time-mapped
    /// voices not yet bounded, so a long look-ahead spreads its evaluation
    /// over several calls; returns whether the whole range is queued.
    pub fn prefetch_within(&mut self, start: i64, frames: usize, budget: usize) -> bool {
        let (spans, complete) = self.source_spans_within(start, frames, budget);
        for (source, lo, hi) in spans {
            self.cache.prefetch_span(&source, lo, (hi - lo) as usize);
        }
        complete
    }

    /// Whether the source blocks for `[start, start + frames)` of every voice
    /// are decoded (playback waits for this before it starts).
    pub fn is_ready(&mut self, start: i64, frames: usize) -> bool {
        self.source_spans(start, frames)
            .iter()
            .all(|(source, lo, hi)| self.cache.is_ready(source, *lo, (hi - lo) as usize))
    }
}

/// The merged source spans a time-mapped voice reads in control-point group
/// `g` (control points `g × READ_GROUP` to `(g + 1) × READ_GROUP`), for a
/// voice heard in project samples `[window.0, window.1)`.
#[allow(clippy::too_many_arguments)]
fn group_spans(
    plan: &VoicePlan,
    fixed: Option<&crate::graph::StaticMap>,
    ev: &mut Evaluator,
    retime: &mut RetimeCache,
    comps: &[pm_eval::Comp<'_>],
    g: i64,
    p: f64,
    window: (i64, i64),
    stretch_block: Option<f64>,
) -> Result<Vec<(i64, i64)>> {
    let open = &comps[0];
    let rate = plan.source.sample_rate() as f64;
    let first = g * READ_GROUP * CONTROL;
    // As `render_voice`: a time-mapped voice sounds where it is in its span.
    let on = |pt: &Point| pt.active && pt.in_span && (stretch_block.is_some() || pt.gain != 0.0);
    let mut spans: Vec<(i64, i64)> = Vec::new();
    let mut prev: Option<Point> = None;
    for k in 0..=READ_GROUP {
        let c = first + k * CONTROL;
        let r = if let Some(fixed) = fixed {
            fixed.evaluate(plan, c as f64 / p)
        } else {
            plan.evaluate(ev, comps, open, c as f64 / p, retime)?
        };
        if let Some(l) = prev
            && (on(&l) || on(&r))
            && c > window.0
            && c - CONTROL < window.1
        {
            let (x0, x1) = (l.source_time * rate, r.source_time * rate);
            if x0.is_finite() && x1.is_finite() {
                // The filter `render_voice` reads this interval with.
                let step = (r.source_time - l.source_time).abs() * rate / CONTROL as f64;
                let taps = kernel::free_taps(step.max(1.0));
                let before = taps / 2 - 1;
                // A stretch reads ahead of its position by its latencies
                // (under two analysis blocks of output), in either
                // direction, through the project-rate reader.
                let pad = stretch_block.map_or(0, |b| {
                    (2.0 * b * step.max(rate / p)).ceil() as i64 + kernel::TAPS as i64
                });
                let lo = x0.min(x1).floor() as i64 - before as i64 - pad;
                let hi = x0.max(x1).floor() as i64 + (taps - before) as i64 + pad;
                spans.push((lo, hi));
            }
        }
        prev = Some(r);
    }
    spans.sort_unstable();
    let mut merged: Vec<(i64, i64)> = Vec::with_capacity(spans.len());
    for (lo, hi) in spans {
        match merged.last_mut() {
            Some(last) if lo <= last.1 => last.1 = last.1.max(hi),
            _ => merged.push((lo, hi)),
        }
    }
    Ok(merged)
}

/// `acc = acc·(1 − w) + next·w`, w ramping from `done` over `len` samples.
fn blend(acc: &mut [Vec<f32>], next: &[Vec<f32>], done: usize, len: f32) {
    for (a, b) in acc.iter_mut().zip(next) {
        for (i, (x, y)) in a.iter_mut().zip(b).enumerate() {
            let w = ((done + i) as f32 / len).min(1.0);
            *x += (*y - *x) * w;
        }
    }
}

/// The project samples `[first, last)` a voice is heard in. A copy reader
/// places the clip on whole samples (source sample `n − shift` plays at
/// project sample n, with `shift = round(offset × rate)`), so its window is
/// measured on that lattice: a clip whose in-point falls between two samples
/// starts with its first source sample at the rounded position instead of
/// losing it to `ceil` (a clip at frame 7 of 29.97 fps, 11,211.2 samples).
fn voice_bounds(reader: &Reader, plan: &VoicePlan, p: f64) -> (i64, i64) {
    match (reader, plan.offset) {
        (Reader::Copy { shift }, Some(offset)) => (
            shift + sample_at(plan.window.0 - offset, p),
            shift + sample_at(plan.window.1 - offset, p),
        ),
        _ => (sample_at(plan.window.0, p), sample_at(plan.window.1, p)),
    }
}

/// The first project sample at or after a time (`ceil(t × rate)`). A time
/// within `1e-9 × max(1, |n|)` of sample n is that sample (amendment 1's
/// tolerance): a clip placed at frame 7 of 25 fps is at 0.28000000000000003 s
/// in f64, and its first source sample must play at sample 13440, where the
/// copy reader's rounded shift puts it, not one sample later (which dropped
/// it).
pub fn sample_at(t: f64, rate: f64) -> i64 {
    if t == f64::INFINITY {
        return i64::MAX / 4;
    }
    if t == f64::NEG_INFINITY {
        return i64::MIN / 4;
    }
    let x = t * rate;
    let n = x.round();
    if (x - n).abs() <= 1e-9 * n.abs().max(1.0) {
        n as i64
    } else {
        x.ceil() as i64
    }
}

/// A reader of the source at the project rate, sample 0 on sample 0 (the
/// input of a stretch).
fn linear_reader(source: u32, project: u32) -> Reader {
    if source == project {
        return Reader::Copy { shift: 0 };
    }
    match kernel::grid_bank(source, project) {
        Some(bank) => {
            let g = kernel::gcd(source as u64, project as u64) as i128;
            Reader::Grid {
                phases: bank.phases as i128,
                bank,
                step: source as i128 / g,
                origin: 0,
            }
        }
        None => Reader::Free,
    }
}

/// Project-rate samples `[lo, lo + n)` of `source` through a linear
/// reader (one vector per source channel, resized to `n`); `src` is
/// scratch. Returns whether every source sample was decoded.
#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
fn read_linear(
    cache: &SourceCache,
    source: &Arc<Source>,
    project: u32,
    reader: &Reader,
    lo: i64,
    n: usize,
    out: &mut [Vec<f32>],
    src: &mut Vec<Vec<f32>>,
    wait: bool,
) -> Result<bool> {
    let sc = source.channels();
    for c in out.iter_mut() {
        c.clear();
        c.resize(n, 0.0);
    }
    if n == 0 {
        return Ok(true);
    }
    match reader {
        Reader::Copy { shift } => {
            let mut views: Vec<&mut [f32]> = out.iter_mut().map(|c| &mut c[..n]).collect();
            cache.read(source, lo - shift, n, &mut views, wait)
        }
        Reader::Grid {
            bank,
            step,
            origin,
            phases,
        } => {
            let unit = |s: i64| *origin + s as i128 * *step;
            let first = unit(lo).div_euclid(*phases) as i64 - bank.before() as i64;
            let last = unit(lo + n as i64 - 1).div_euclid(*phases) as i64
                + (bank.taps - bank.before()) as i64;
            let span = (last - first) as usize;
            src.resize_with(sc, Vec::new);
            for c in src.iter_mut() {
                c.clear();
                c.resize(span, 0.0);
            }
            let mut views: Vec<&mut [f32]> = src.iter_mut().map(|c| &mut c[..span]).collect();
            let complete = cache.read(source, first, span, &mut views, wait)?;
            crate::realtime::checkpoint_phase("read-linear-grid");
            for i in 0..n {
                if i.is_multiple_of(64) {
                    crate::realtime::checkpoint_dsp_step();
                }
                let u = unit(lo + i as i64);
                let (si, ph) = (u.div_euclid(*phases) as i64, u.rem_euclid(*phases) as usize);
                for c in 0..sc {
                    out[c][i] = bank.grid(&src[c], first, si, ph);
                }
            }
            Ok(complete)
        }
        Reader::Free => {
            // A rate pair without an exact-phase bank: free positions.
            let ratio = source.sample_rate() as f64 / project as f64;
            let bank = kernel::free_bank(ratio);
            let pos = |i: usize| (lo + i as i64) as f64 * ratio;
            let first = pos(0).floor() as i64 - bank.before() as i64;
            let last = pos(n - 1).floor() as i64 + (bank.taps - bank.before()) as i64;
            let span = (last - first) as usize;
            src.resize_with(sc, Vec::new);
            for c in src.iter_mut() {
                c.clear();
                c.resize(span, 0.0);
            }
            let mut views: Vec<&mut [f32]> = src.iter_mut().map(|c| &mut c[..span]).collect();
            let complete = cache.read(source, first, span, &mut views, wait)?;
            let mut kern = vec![0f32; bank.taps];
            crate::realtime::checkpoint_phase("read-linear-free");
            for i in 0..n {
                if i.is_multiple_of(64) {
                    crate::realtime::checkpoint_dsp_step();
                }
                let x = pos(i);
                let fl = x.floor();
                bank.free_kernel(x - fl, &mut kern);
                let at = (fl as i64 - bank.before() as i64 - first) as usize;
                for c in 0..sc {
                    out[c][i] = kernel::dot(&src[c][at..at + bank.taps], &kern);
                }
            }
            Ok(complete)
        }
    }
}

fn reader_for(plan: &VoicePlan, project: u32) -> Reader {
    let source = plan.source.sample_rate();
    let Some(offset) = plan.offset else {
        return Reader::Free;
    };
    if source == project {
        return Reader::Copy {
            shift: (offset * project as f64).round() as i64,
        };
    }
    match kernel::grid_bank(source, project) {
        Some(bank) => {
            let g = kernel::gcd(source as u64, project as u64) as i128;
            let phases = bank.phases as i128;
            // Output n reads source position (n / project − offset) × source,
            // in units of 1/phases: n × source/g − offset × source × phases.
            let origin = -((offset * source as f64 * phases as f64).round() as i128);
            Reader::Grid {
                bank,
                step: source as i128 / g,
                origin,
                phases,
            }
        }
        None => Reader::Free,
    }
}

#[cfg(test)]
mod prepared_tests {
    use super::*;
    use crate::offline::{PcmFormat, WavWriter};
    use pm_model::json::parse;

    struct Fixture {
        cache: Arc<SourceCache>,
        dir: tempfile::TempDir,
    }
    impl Fixture {
        fn new() -> Self {
            let dir = tempfile::Builder::new()
                .prefix("pm-audio-prepared-")
                .tempdir()
                .unwrap();
            let mut w = WavWriter::new(
                std::fs::File::create(dir.path().join("tone.wav")).unwrap(),
                PcmFormat::F32,
                2,
                48_000,
            )
            .unwrap();
            let pcm: Vec<f32> = (0..48_000 * 12)
                .flat_map(|i| {
                    let t = i as f64 / 48_000.0;
                    [
                        (0.2 * (t * 997.0 * std::f64::consts::TAU).sin()) as f32,
                        (0.1 * (t * 631.0 * std::f64::consts::TAU).sin()) as f32,
                    ]
                })
                .collect();
            w.write(0, &pcm).unwrap();
            w.finish().unwrap();
            let cache = SourceCache::new(64 << 20, 0);
            let source = cache.open(&dir.path().join("tone.wav"), None).unwrap();
            for b in 0..9 {
                cache.block(&source, b, true).unwrap();
            }
            Self { cache, dir }
        }
        fn graph(&self, gain: f64, speed: f64, dur: f64) -> (Arc<Document>, Arc<MixGraph>) {
            self.mapped_graph(gain, &speed.to_string(), dur)
        }
        fn mapped_graph(&self, gain: f64, speed: &str, dur: f64) -> (Arc<Document>, Arc<MixGraph>) {
            let layers: Vec<String> = (0..4).map(|i| format!(r#"{{"id":"v{i}","type":"audio","from":0,"dur":{dur},"on":true,"d":{{"asset":"s","gain":{gain},"trim":0,"fadeIn":0,"fadeOut":0,"speed":{speed}}}}}"#)).collect();
            let doc = Arc::new(Document::new(
                Some(
                    parse(&format!(
                        r#"{{"id":"P","compId":"C","fps":30,"dur":10,"layers":[{}],"comps":{{}}}}"#,
                        layers.join(",")
                    ))
                    .unwrap(),
                ),
                1,
            ));
            let graph = Arc::new(
                MixGraph::build(&doc, "C", &mut Evaluator::new(), &self.cache, &|_: &str| {
                    Some(self.dir.path().join("tone.wav"))
                })
                .unwrap(),
            );
            (doc, graph)
        }
        fn mixer(&self, gain: f64, speed: f64, dur: f64) -> Mixer {
            let (doc, graph) = self.graph(gain, speed, dur);
            Mixer::new(
                MixFormat::stereo(48_000),
                doc,
                graph,
                Evaluator::new(),
                self.cache.clone(),
            )
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.cache.shutdown();
        }
    }
    fn prepare(m: &mut Mixer, start: i64) {
        m.mark_unstretched_ready(start);
        let mut workers = crate::preparation::Preparation::new();
        workers.evaluators(Arc::new(Evaluator::new), false);
        workers.schedule(m, &[start], true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !m.prepared_ready(&[start]) && std::time::Instant::now() < deadline {
            assert!(
                !workers.pending().is_empty(),
                "workers finished without complete preparation at {start}"
            );
            workers.wait(m, std::time::Duration::from_millis(10));
        }
        assert!(m.prepared_ready(&[start]), "worker preparation at {start}");
    }

    #[test]
    fn installed_graph_remaps_prepared_job_criticality_by_signal_identity() {
        let f = Fixture::new();
        let build = |voices: &[(&str, f64, f64)]| {
            let layers = voices
                .iter()
                .map(|(id, speed, gain)| {
                    format!(
                        r#"{{"id":"{id}","type":"audio","from":0,"dur":8,"on":true,"d":{{"asset":"s","gain":{gain},"trim":0,"fadeIn":0,"fadeOut":0,"speed":{speed}}}}}"#
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            let doc = Arc::new(Document::new(
                Some(
                    parse(&format!(
                        r#"{{"id":"P","compId":"C","fps":30,"dur":10,"layers":[{layers}],"comps":{{}}}}"#
                    ))
                    .unwrap(),
                ),
                1,
            ));
            let graph = Arc::new(
                MixGraph::build(&doc, "C", &mut Evaluator::new(), &f.cache, &|_: &str| {
                    Some(f.dir.path().join("tone.wav"))
                })
                .unwrap(),
            );
            (doc, graph)
        };
        let start = 4 * 48_000;
        let (doc, graph) = build(&[]);
        let mut mix = Mixer::new(
            MixFormat::stereo(48_000),
            doc,
            graph,
            Evaluator::new(),
            f.cache.clone(),
        );
        mix.mark_unstretched_ready(start);
        assert!(mix.prepared[&start].critical.is_empty());

        let (doc, graph) = build(&[("a", 1.5, 0.5), ("b", 1.5, 0.5)]);
        mix.set_graph(doc, graph);
        assert_eq!(mix.prepared[&start].critical.len(), 2);
        let queued = mix.preparation_work_except(start, start, |_| false);
        assert_eq!(
            queued.len(),
            2,
            "new stretched voices have independent jobs"
        );
        let a = queued
            .iter()
            .find(|job| job.vi == 0)
            .unwrap()
            .critical
            .clone();
        let b = queued
            .iter()
            .find(|job| job.vi == 1)
            .unwrap()
            .critical
            .clone();
        a.store(true, Ordering::Release);

        // Reordering and gain-only edits preserve each queued job's flag.
        let (doc, graph) = build(&[("b", 1.5, 0.7), ("a", 1.5, 0.7)]);
        mix.set_graph(doc, graph);
        assert!(Arc::ptr_eq(&mix.prepared[&start].critical[0], &b));
        assert!(Arc::ptr_eq(&mix.prepared[&start].critical[1], &a));
        assert!(mix.prepared[&start].critical[1].load(Ordering::Acquire));

        // A map change invalidates a's old promotion; b's unchanged signal
        // still carries its queued job's flag. A newly added voice is false.
        b.store(true, Ordering::Release);
        let (doc, graph) = build(&[("b", 1.5, 0.7), ("a", 1.25, 0.7), ("c", 1.5, 0.7)]);
        mix.set_graph(doc, graph);
        let flags = &mix.prepared[&start].critical;
        assert_eq!(flags.len(), 3);
        assert!(Arc::ptr_eq(&flags[0], &b));
        assert!(!Arc::ptr_eq(&flags[1], &a));
        assert!(flags[0].load(Ordering::Acquire));
        assert!(!flags[1].load(Ordering::Acquire));
        assert!(!flags[2].load(Ordering::Acquire));

        let fresh = mix.preparation_work_except(start, start, |_| false);
        assert_eq!(fresh.len(), 3);
        for job in &fresh {
            assert!(
                !Arc::ptr_eq(&job.history(), mix.voices[job.vi].stretch.as_ref().unwrap(),),
                "a detached job cannot own the live history"
            );
        }
    }
    #[test]
    fn a_partial_preparation_does_not_release_other_cold_voices() {
        let f = Fixture::new();
        let mut mix = f.mixer(1.0, 1.5, 8.0);
        mix.mark_unstretched_ready(9 * 48_000);
        assert!(!mix.stretches_ready(9 * 48_000));
        let start = 4 * 48_000;
        mix.mark_unstretched_ready(start);
        let jobs = mix.preparation_work(start, start);
        assert_eq!(jobs.len(), 4);
        let first = jobs.into_iter().next().unwrap();
        let vi = first.vi;
        assert!(first.claim());
        assert!(first.run(&mut Evaluator::new(), false, || false).unwrap());
        mix.prepared_voice(start, vi, true);
        assert_eq!(mix.prepared[&start].ready.iter().filter(|&&v| v).count(), 1);
        assert!(!mix.prepared_ready(&[start]));
        assert!(!mix.stretches_ready(start));
        assert!(!mix.stretches_ready(9 * 48_000));
        let mut muted = f.mixer(0.0, 1.5, 8.0);
        muted.mark_unstretched_ready(start);
        assert!(!muted.stretches_ready(start));
    }

    #[test]
    fn cached_unstretched_voices_need_no_vocoder_jobs() {
        let f = Fixture::new();
        let mut mix = f.mixer(1.0, 1.0, 8.0);
        let start = 4 * 48_000;
        assert!(mix.voices.iter().all(|v| v.stretch.is_none()));
        mix.mark_unstretched_ready(start);
        assert!(mix.prepared_ready(&[start]));
        assert!(mix.preparation_work(start, start).is_empty());
    }

    #[test]
    fn detached_jobs_use_custom_channel_hooks() {
        struct Hooks;
        impl pm_eval::EvalHooks for Hooks {
            fn resolve_content(
                &self,
                _: &pm_eval::Comp<'_>,
                _: &pm_model::Node,
                _: f64,
                content: &mut pm_eval::evaluator::Content,
                _: &mut dyn pm_eval::evaluator::PropEval,
            ) -> pm_eval::js_sem::JsResult<()> {
                content.insert(
                    pm_model::JsString::from("speed"),
                    pm_eval::js_sem::JsValue::Number(1.25),
                );
                Ok(())
            }
        }

        let ev = || Evaluator::new().with_hooks(Arc::new(Hooks));
        let f = Fixture::new();
        let make = || {
            let (doc, graph) = f.mapped_graph(0.3, r#"{"v":1.5,"kf":[],"expr":""}"#, 10.0);
            Mixer::new(MixFormat::stereo(48_000), doc, graph, ev(), f.cache.clone())
        };
        let mut prepared = make();
        for job in prepared.preparation_work(192_000, 192_000) {
            let vi = job.vi;
            assert!(job.run(&mut ev(), false, || false).unwrap());
            prepared.prepared_voice(192_000, vi, true);
        }
        prepared.use_prepared(192_000);
        let mut actual = vec![vec![]; 2];
        let mut expected = vec![vec![]; 2];
        let options = RenderOptions {
            wait: true,
            ..Default::default()
        };
        prepared
            .render(192_000, 4096, &mut actual, &options)
            .unwrap();
        make()
            .render(192_000, 4096, &mut expected, &options)
            .unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn immutable_heads_start_complete_then_handoff_to_independent_histories() {
        let f = Fixture::new();
        let mut mix = f.mixer(0.7, 1.5, 10.0);
        mix.standard_content(true);
        let start = 192_000;
        let mut jobs = mix.preparation_work(start, start).into_iter();
        let first = jobs.next().unwrap();
        assert!(first.run(&mut Evaluator::new(), true, || false).unwrap());
        mix.prepared_voice(start, 0, true);
        assert!(mix.prepared_ready(&[start]), "all voices have complete PCM");
        assert!(
            !mix.prepared[&start].ready[1],
            "other histories deliberately not run yet"
        );
        let heads = &mix.prepared[&start].heads;
        assert!(
            heads
                .iter()
                .all(|h| Arc::ptr_eq(h.as_ref().unwrap(), heads[0].as_ref().unwrap()))
        );
        mix.use_prepared(start);
        assert!(mix.awaiting_history(start + HEAD_FRAMES as i64, 256));
        let mut reference = f.mixer(0.7, 1.5, 10.0);
        let mut actual = vec![vec![]; 2];
        let mut expected = vec![vec![]; 2];
        let options = RenderOptions {
            wait: true,
            ..Default::default()
        };
        for offset in (0..HEAD_FRAMES).step_by(256) {
            mix.render(start + offset as i64, 256, &mut actual, &options)
                .unwrap();
            reference
                .render(start + offset as i64, 256, &mut expected, &options)
                .unwrap();
            assert_eq!(actual, expected, "prefilled complete mix at {offset}");
        }
        for job in jobs {
            let vi = job.vi;
            let ready = if vi == 1 {
                // v1's job met an evicted input block: it reports not ready
                // and releases its history unadvanced, as `run` does.
                assert!(job.claim());
                job.finished.store(true, Ordering::Release);
                false
            } else {
                job.run(&mut Evaluator::new(), true, || false).unwrap()
            };
            mix.prepared_voice(start, vi, ready);
        }
        assert!(!mix.awaiting_history(start + HEAD_FRAMES as i64, 256));
        assert!(mix.voices.iter().enumerate().all(|(i, v)| {
            mix.voices.iter().enumerate().all(|(j, w)| {
                i == j || !Arc::ptr_eq(v.stretch.as_ref().unwrap(), w.stretch.as_ref().unwrap())
            })
        }));
        for offset in (HEAD_FRAMES..HEAD_FRAMES + 8192).step_by(256) {
            mix.render(start + offset as i64, 256, &mut actual, &options)
                .unwrap();
            reference
                .render(start + offset as i64, 256, &mut expected, &options)
                .unwrap();
            assert_eq!(actual, expected, "independent handoff at {offset}");
        }
    }

    #[test]
    fn a_live_level_edit_keeps_the_prefilled_phase_without_waiting_on_cancelled_jobs() {
        let f = Fixture::new();
        let start = 196_480;
        let mut actual = f.mixer(0.3, 1.5, 10.0);
        actual.standard_content(true);
        let mut jobs = actual.preparation_work(start, start).into_iter();
        assert!(
            jobs.next()
                .unwrap()
                .run(&mut Evaluator::new(), true, || false)
                .unwrap()
        );
        actual.prepared_voice(start, 0, true);
        actual.use_prepared(start);
        let mut reference = f.mixer(0.3, 1.5, 10.0);
        let options = RenderOptions {
            wait: true,
            ..Default::default()
        };
        let mut a = vec![vec![]; 2];
        let mut b = vec![vec![]; 2];
        actual.render(start, 512, &mut a, &options).unwrap();
        reference.render(start, 512, &mut b, &options).unwrap();
        assert_eq!(a, b);
        let (doc, graph) = f.graph(0.8, 1.5, 10.0);
        actual.glide_to(doc.clone(), graph.clone(), None);
        reference.glide_to(doc, graph, None);
        assert!(
            !actual.awaiting_history(start + HEAD_FRAMES as i64, 256),
            "a live edit cancels the detached jobs, so their flags cannot gate the producer"
        );
        for offset in (512..HEAD_FRAMES + 4096).step_by(256) {
            actual
                .render(start + offset as i64, 256, &mut a, &options)
                .unwrap();
            reference
                .render(start + offset as i64, 256, &mut b, &options)
                .unwrap();
            assert_eq!(a, b, "live edit at {offset}");
        }
    }

    #[test]
    fn replacing_the_evaluator_never_reuses_a_previous_factories_pcm() {
        struct Hooks;
        impl pm_eval::EvalHooks for Hooks {
            fn resolve_content(
                &self,
                _: &pm_eval::Comp<'_>,
                _: &pm_model::Node,
                _: f64,
                d: &mut pm_eval::evaluator::Content,
                _: &mut dyn pm_eval::evaluator::PropEval,
            ) -> pm_eval::js_sem::JsResult<()> {
                d.insert(
                    pm_model::JsString::from("speed"),
                    pm_eval::js_sem::JsValue::Number(1.25),
                );
                Ok(())
            }
        }
        let f = Fixture::new();
        let start = 196_480;
        let mut actual = f.mixer(0.7, 1.5, 10.0);
        actual.standard_content(true);
        let first = actual.preparation_work(start, start).remove(0);
        assert!(first.run(&mut Evaluator::new(), true, || false).unwrap());
        actual.prepared_voice(start, 0, true);
        actual.use_prepared(start);
        let options = RenderOptions {
            wait: true,
            ..Default::default()
        };
        let mut a = vec![vec![]; 2];
        let mut b = vec![vec![]; 2];
        actual.render(start, 512, &mut a, &options).unwrap();
        let evaluator = || Evaluator::new().with_hooks(Arc::new(Hooks));
        actual.set_evaluator(evaluator());
        let (doc, graph) = f.graph(0.7, 1.5, 10.0);
        let mut reference = Mixer::new(
            MixFormat::stereo(48_000),
            doc,
            graph,
            evaluator(),
            f.cache.clone(),
        );
        actual.render(start + 512, 4096, &mut a, &options).unwrap();
        reference
            .render(start + 512, 4096, &mut b, &options)
            .unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn immutable_head_identity_does_not_merge_different_signals() {
        let f = Fixture::new();
        let layers = (0..4).map(|i| format!(r#"{{"id":"v{i}","type":"audio","from":0,"dur":10,"on":true,"d":{{"asset":"s","gain":{},"fadeIn":{},"trim":{},"speed":{}}}}}"#,
            0.2 + i as f64 * 0.1, i as f64 * 0.2, if i==2 {0.1} else {0.0}, if i==3 {1.6} else {1.5})).collect::<Vec<_>>();
        let doc = Arc::new(Document::new(
            Some(
                parse(&format!(
                    r#"{{"id":"P","compId":"C","fps":30,"dur":10,"layers":[{}],"comps":{{}}}}"#,
                    layers.join(",")
                ))
                .unwrap(),
            ),
            1,
        ));
        let graph = Arc::new(
            MixGraph::build(&doc, "C", &mut Evaluator::new(), &f.cache, &|_: &str| {
                Some(f.dir.path().join("tone.wav"))
            })
            .unwrap(),
        );
        let mut mix = Mixer::new(
            MixFormat::stereo(48_000),
            doc.clone(),
            graph.clone(),
            Evaluator::new(),
            f.cache.clone(),
        );
        mix.standard_content(true);
        let start = 192_000;
        let jobs = mix.preparation_work(start, start);
        let h = &mix.prepared[&start].heads;
        assert!(
            Arc::ptr_eq(h[0].as_ref().unwrap(), h[1].as_ref().unwrap()),
            "gain and fade apply after raw PCM"
        );
        for i in [2, 3] {
            assert!(h[i].is_none(), "unique signals do not compute unused heads");
        }
        let mut jobs = jobs.into_iter();
        assert!(
            jobs.next()
                .unwrap()
                .run(&mut Evaluator::new(), true, || false)
                .unwrap()
        );
        mix.prepared_voice(start, 0, true);
        assert!(mix.begin_prepared(start));
        assert!(
            mix.preparing_ids().is_empty(),
            "A3 never omits a voice whose own input is ready"
        );
        assert!(
            mix.voices[1].prefill.is_some(),
            "A3 must retain v1's complete head while its history is pending"
        );
        for job in jobs {
            let vi = job.vi;
            assert!(job.run(&mut Evaluator::new(), true, || false).unwrap());
            mix.prepared_voice(start, vi, true);
        }
        mix.clear_preparing();
        mix.use_prepared(start);
        let mut reference = Mixer::new(
            MixFormat::stereo(48_000),
            doc,
            graph,
            Evaluator::new(),
            f.cache.clone(),
        );
        let options = RenderOptions {
            wait: true,
            ..Default::default()
        };
        let mut actual = vec![vec![]; 2];
        let mut expected = vec![vec![]; 2];
        mix.render(start, 8192, &mut actual, &options).unwrap();
        reference
            .render(start, 8192, &mut expected, &options)
            .unwrap();
        assert_eq!(actual, expected);
    }

    /// A3's safety net is per voice: a stalled decoder delays or omits only
    /// its own voice. Voices whose input is ready but whose CPU job is still
    /// queued start complete (from a head, or primed by the ordinary render).
    #[test]
    fn the_a3_safety_net_omits_only_voices_whose_own_input_is_missing() {
        let f = Fixture::new();
        let late = f.dir.path().join("late.wav");
        let mut w = WavWriter::new(
            std::fs::File::create(&late).unwrap(),
            PcmFormat::F32,
            2,
            48_000,
        )
        .unwrap();
        w.write(0, &vec![0.25f32; 2 * 48_000 * 10]).unwrap();
        w.finish().unwrap();
        let layer = |i: usize, asset: &str, trim: f64| {
            format!(
                r#"{{"id":"v{i}","type":"audio","from":0,"dur":10,"on":true,"d":{{"asset":"{asset}","gain":0.5,"fadeIn":0,"fadeOut":0,"trim":{trim},"speed":1.5}}}}"#
            )
        };
        let build = |layers: &[String]| {
            let doc = Arc::new(Document::new(
                Some(
                    parse(&format!(
                        r#"{{"id":"P","compId":"C","fps":30,"dur":10,"layers":[{}],"comps":{{}}}}"#,
                        layers.join(",")
                    ))
                    .unwrap(),
                ),
                1,
            ));
            let graph = Arc::new(
                MixGraph::build(
                    &doc,
                    "C",
                    &mut Evaluator::new(),
                    &f.cache,
                    &|asset: &str| {
                        Some(if asset == "late" {
                            late.clone()
                        } else {
                            f.dir.path().join("tone.wav")
                        })
                    },
                )
                .unwrap(),
            );
            Mixer::new(
                MixFormat::stereo(48_000),
                doc,
                graph,
                Evaluator::new(),
                f.cache.clone(),
            )
        };
        let all = [
            layer(0, "s", 0.0),
            layer(1, "s", 0.0),
            layer(2, "s", 0.1),
            layer(3, "late", 0.0),
        ];
        let mut mix = build(&all);
        mix.standard_content(true);
        let start = 192_000;
        // The zero-worker cache never decodes the late source.
        assert!(!mix.is_ready(start, crate::ring::CHUNK));
        let mut jobs = mix.preparation_work(start, start).into_iter();
        let first = jobs.next().unwrap();
        assert_eq!(first.vi, 0);
        assert!(first.run(&mut Evaluator::new(), true, || false).unwrap());
        mix.prepared_voice(start, 0, true);
        // v1 shares v0's head; v2's job and v3's job never ran.
        assert!(mix.begin_prepared(start));
        assert_eq!(mix.preparing_ids(), ["v3".to_string()].into());
        // The late start's joins are scheduled from the producer (target
        // past the start). v1's history left the entry with its head: no
        // job is scheduled on what the audible mix plays.
        let live: Vec<_> = mix
            .voices
            .iter()
            .map(|v| v.stretch.clone().unwrap())
            .collect();
        let late = mix.preparation_work_except(start, start + 256, |_| false);
        let mut voices: Vec<usize> = late.iter().map(|j| j.vi).collect();
        voices.sort();
        assert_eq!(voices, [2, 3]);
        for job in &late {
            assert!(live.iter().all(|l| !Arc::ptr_eq(l, &job.history())));
        }
        let options = RenderOptions {
            drop_voices: mix.preparing_ids(),
            ..Default::default()
        };
        let mut reference = build(&all[..3]);
        let mut actual = vec![vec![]; 2];
        let mut expected = vec![vec![]; 2];
        for offset in (0..8192).step_by(256) {
            mix.render(start + offset, 256, &mut actual, &options)
                .unwrap();
            reference
                .render(
                    start + offset,
                    256,
                    &mut expected,
                    &RenderOptions {
                        wait: true,
                        ..Default::default()
                    },
                )
                .unwrap();
            assert_eq!(actual, expected, "ready voices complete at {offset}");
        }
    }

    /// At the end of a complete shared head, the producer waits for every
    /// independent history. A queued job remains queued; only its worker may
    /// prime it, and the handoff must retain the exact offline PCM.
    #[test]
    fn an_exhausted_head_waits_for_queued_and_running_histories() {
        let f = Fixture::new();
        let mut mix = f.mixer(0.7, 1.5, 10.0);
        mix.standard_content(true);
        let start = 192_000;
        let mut jobs = mix.preparation_work(start, start).into_iter();
        let first = jobs.next().unwrap();
        assert!(first.claim());
        assert!(first.run(&mut Evaluator::new(), true, || false).unwrap());
        mix.prepared_voice(start, 0, true);
        let rest: Vec<_> = jobs.collect();
        assert_eq!(rest.iter().map(|j| j.vi).collect::<Vec<_>>(), [1, 2, 3]);
        // v1 has been taken by a worker. v2 and v3 are still queued.
        assert!(rest[0].claim());
        assert!(
            rest[1..]
                .iter()
                .all(|job| !job.started.load(Ordering::Acquire))
        );
        mix.use_prepared(start);
        let mut reference = f.mixer(0.7, 1.5, 10.0);
        let options = RenderOptions {
            wait: true,
            ..Default::default()
        };
        let mut actual = vec![vec![]; 2];
        let mut expected = vec![vec![]; 2];
        let mut compare = |mix: &mut Mixer, from: usize, to: usize| {
            for offset in (from..to).step_by(256) {
                mix.render(start + offset as i64, 256, &mut actual, &options)
                    .unwrap();
                reference
                    .render(start + offset as i64, 256, &mut expected, &options)
                    .unwrap();
                assert_eq!(actual, expected, "complete mix at {offset}");
            }
        };
        compare(&mut mix, 0, HEAD_FRAMES);
        let end = start + HEAD_FRAMES as i64;
        assert!(mix.awaiting_history(end, 256));
        for (i, job) in rest.into_iter().enumerate() {
            if i > 0 {
                assert!(
                    job.claim(),
                    "queued v{} still belongs to its worker",
                    job.vi
                );
            }
            let vi = job.vi;
            let ready = job.run(&mut Evaluator::new(), true, || false).unwrap();
            assert!(ready, "worker v{vi} completed its independent history");
            assert_eq!(
                mix.awaiting_history(end, 256),
                i < 2,
                "head handoff after worker v{vi}"
            );
        }
        assert!(!mix.awaiting_history(end, 256));
        compare(&mut mix, HEAD_FRAMES, HEAD_FRAMES + 8192);
    }

    /// The bit-exact comparison the handoff tests share: `mix` against a fresh
    /// mixer rendering the same frames from `start` on.
    struct Reference {
        mix: Mixer,
        actual: Vec<Vec<f32>>,
        expected: Vec<Vec<f32>>,
    }
    impl Reference {
        fn new(f: &Fixture) -> Self {
            Self {
                mix: f.mixer(0.7, 1.5, 10.0),
                actual: vec![vec![]; 2],
                expected: vec![vec![]; 2],
            }
        }
        fn compare(&mut self, mix: &mut Mixer, start: i64, from: usize, to: usize) {
            let options = RenderOptions {
                wait: true,
                ..Default::default()
            };
            for offset in (from..to).step_by(256) {
                let at = start + offset as i64;
                assert!(
                    !mix.awaiting_history(at, 256),
                    "worker histories must be ready at {offset}"
                );
                mix.render(at, 256, &mut self.actual, &options).unwrap();
                self.mix
                    .render(at, 256, &mut self.expected, &options)
                    .unwrap();
                assert_eq!(self.actual, self.expected, "complete mix at {offset}");
            }
        }
    }

    #[test]
    fn cancelled_continuation_cannot_reset_a_history_the_producer_advanced() {
        use std::sync::atomic::AtomicUsize;
        use std::time::Duration;

        let f = Fixture::new();
        let start = 192_000;
        let mut mix = f.mixer(0.7, 1.5, 10.0);
        mix.standard_content(true);
        let job = mix.preparation_work(start, start).remove(0);
        let history = job.history();
        let finished = job.finished.clone();
        assert!(job.claim());
        let mut task = job.into_task(Evaluator::new(), true);
        assert!(matches!(task.step(|| false).unwrap(), PrepareStep::Yielded));

        // The producer owns the mutex while the continuation's first
        // cancellation check still observes the old generation.
        let mut producer = history.lock().unwrap();
        let cancelled = Arc::new(AtomicBool::new(false));
        let worker_cancelled = cancelled.clone();
        let checks = AtomicUsize::new(0);
        let (checked_tx, checked_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            task.step(|| {
                let old = worker_cancelled.load(Ordering::Relaxed);
                if checks.fetch_add(1, Ordering::Relaxed) == 0 {
                    checked_tx.send(()).unwrap();
                }
                old
            })
            .map(|_| ())
        });
        checked_rx.recv_timeout(Duration::from_secs(2)).unwrap();

        let at = start + 2 * stretch::GRID;
        let mut map = |k: i64| {
            Ok(MapPoint {
                x: (k * CONTROL) as f64 * 1.5,
                on: true,
            })
        };
        let mut read = |lo: i64, n: usize, out: &mut [Vec<f32>]| {
            for (channel, samples) in out.iter_mut().enumerate() {
                samples.clear();
                samples.extend(
                    (0..n).map(|i| ((lo + i as i64) as f64 * 0.017 + channel as f64).sin() as f32),
                );
            }
            Ok(true)
        };
        let mut actual = vec![vec![]; 2];
        let mut expected = vec![vec![]; 2];
        let mut reference = StretchRun::new(2, 48_000, producer.key());
        assert!(
            producer
                .render(at, 256, &mut map, &mut read, &mut actual)
                .unwrap()
        );
        assert!(
            reference
                .render(at, 256, &mut map, &mut read, &mut expected)
                .unwrap()
        );
        let starts = producer.fresh_starts;
        // This bump precedes the producer's mutex release. A post-lock
        // check must observe it before fresh() can destroy the live phase.
        cancelled.store(true, Ordering::Release);
        drop(producer);
        assert!(worker.join().unwrap().is_err());
        assert!(finished.load(Ordering::Acquire));
        let mut producer = history.lock().unwrap();
        assert_eq!(
            producer.fresh_starts, starts,
            "cancelled task never called fresh"
        );
        assert!(
            producer
                .render(at + 256, 4096, &mut map, &mut read, &mut actual)
                .unwrap()
        );
        assert!(
            reference
                .render(at + 256, 4096, &mut map, &mut read, &mut expected)
                .unwrap()
        );
        for (a, b) in actual.iter().zip(&expected) {
            assert_eq!(
                a.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                b.iter().map(|x| x.to_bits()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn solo_handoff_between_ownership_check_and_history_lock_retires_worker() {
        use std::time::Duration;

        let f = Fixture::new();
        let start = 192_000;
        let (mut mix, jobs) = partly_installed(&f, start);
        let job = jobs.into_iter().find(|job| job.vi == 1).unwrap();
        let history = job.history();
        let finished = job.finished.clone();
        assert!(job.claim());
        let mut task = job.into_task(Evaluator::new(), true);
        assert!(matches!(task.step(|| false).unwrap(), PrepareStep::Yielded));
        assert!(!finished.load(Ordering::Acquire));

        let mut producer = history.lock().unwrap();
        let (at_lock_tx, at_lock_rx) = std::sync::mpsc::channel();
        task.before_history_lock = Some(Box::new(move || at_lock_tx.send(()).unwrap()));
        let worker = std::thread::spawn(move || task.step(|| false));
        at_lock_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!finished.load(Ordering::Acquire));

        // Solo's same-graph glide transfers the installed prefill to the
        // producer without invalidating the preparation generation. The
        // worker has already passed its first finished check and is blocked
        // on this exact run's mutex.
        mix.glide_to(mix.doc.clone(), mix.graph.clone(), None);
        assert!(finished.load(Ordering::Acquire));
        let at = start + 2 * stretch::GRID;
        let mut map = |k: i64| {
            Ok(MapPoint {
                x: (k * CONTROL) as f64 * 1.5,
                on: true,
            })
        };
        let mut read = |lo: i64, n: usize, out: &mut [Vec<f32>]| {
            for (channel, samples) in out.iter_mut().enumerate() {
                samples.clear();
                samples.extend(
                    (0..n).map(|i| ((lo + i as i64) as f64 * 0.017 + channel as f64).sin() as f32),
                );
            }
            Ok(true)
        };
        let mut actual = vec![vec![]; 2];
        let mut expected = vec![vec![]; 2];
        let mut reference = StretchRun::new(2, 48_000, producer.key());
        assert!(
            producer
                .render(at, 256, &mut map, &mut read, &mut actual)
                .unwrap()
        );
        assert!(
            reference
                .render(at, 256, &mut map, &mut read, &mut expected)
                .unwrap()
        );
        let starts = producer.fresh_starts;
        drop(producer);

        assert!(matches!(
            worker.join().unwrap().unwrap(),
            PrepareStep::Retired
        ));
        assert_eq!(history.lock().unwrap().fresh_starts, starts);
        let mut producer = history.lock().unwrap();
        assert!(
            producer
                .render(at + 256, 4096, &mut map, &mut read, &mut actual)
                .unwrap()
        );
        assert!(
            reference
                .render(at + 256, 4096, &mut map, &mut read, &mut expected)
                .unwrap()
        );
        for (a, b) in actual.iter().zip(&expected) {
            assert_eq!(
                a.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                b.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                "the live producer phase survives the retiring worker"
            );
        }
    }

    #[test]
    fn solo_edit_retires_a_sliced_history_after_the_producer_takes_it() {
        let f = Fixture::new();
        let start = 192_000;
        let (mut actual, rest) = partly_installed(&f, start);
        let mut reference = f.mixer(0.7, 1.5, 10.0);
        let mut rest = rest.into_iter();
        let running = rest.next().unwrap();
        let queued = rest.next().unwrap();
        assert_eq!((running.vi, queued.vi), (1, 2));
        let history = running.history();
        let queued_history = queued.history();
        let finished = running.finished.clone();
        let queued_finished = queued.finished.clone();
        assert!(running.claim());
        let mut task = running.into_task(Evaluator::new(), true);
        assert!(matches!(task.step(|| false).unwrap(), PrepareStep::Yielded));
        assert!(!finished.load(Ordering::Acquire));
        assert!(history.try_lock().is_ok(), "yield releases the live run");
        assert!(!queued.started.load(Ordering::Acquire));

        let plain = RenderOptions {
            wait: true,
            ..Default::default()
        };
        let mut a = vec![vec![]; 2];
        let mut b = vec![vec![]; 2];
        actual.render(start, 256, &mut a, &plain).unwrap();
        reference.render(start, 256, &mut b, &plain).unwrap();
        assert_eq!(a, b);

        // Control::Solo glides the same graph while changing the monitor
        // selection. install() retires the prefills without bumping the
        // preparation generation: the queued task must see that ownership
        // handoff before it can resume its old start.
        let solo = HashSet::from([actual.graph.voices[1].strip.clone()]);
        let (doc, graph) = (actual.doc.clone(), actual.graph.clone());
        actual.glide_to(doc, graph, None);
        let (doc, graph) = (reference.doc.clone(), reference.graph.clone());
        reference.glide_to(doc, graph, None);
        assert!(finished.load(Ordering::Acquire));
        assert!(queued_finished.load(Ordering::Acquire));
        assert!(!actual.awaiting_history(start + HEAD_FRAMES as i64, 256));
        let solo_options = RenderOptions {
            wait: true,
            monitor_solo: Some(solo),
            ..Default::default()
        };
        let bits = |pcm: &[Vec<f32>]| {
            pcm.iter()
                .flat_map(|channel| channel.iter().map(|sample| sample.to_bits()))
                .collect::<Vec<_>>()
        };
        for offset in (256..HEAD_FRAMES + 8192).step_by(256) {
            let at = start + offset as i64;
            actual.render(at, 256, &mut a, &solo_options).unwrap();
            reference.render(at, 256, &mut b, &solo_options).unwrap();
            assert_eq!(bits(&a), bits(&b), "Solo PCM at {offset}");
        }
        let producer_at = start + (HEAD_FRAMES + 8192 - 256) as i64;
        let run = history.lock().unwrap();
        assert!(
            run.can_continue_at(producer_at),
            "the producer advanced the live history past the head"
        );
        let starts = run.fresh_starts;
        drop(run);
        assert!(matches!(task.step(|| false).unwrap(), PrepareStep::Retired));
        assert_eq!(history.lock().unwrap().fresh_starts, starts);

        // A job that was still queued at the edit must not initialize its
        // run either, even if its old claim is later dispatched.
        let queued_starts = queued_history.lock().unwrap().fresh_starts;
        assert!(queued.claim());
        let mut queued_task = queued.into_task(Evaluator::new(), true);
        assert!(matches!(
            queued_task.step(|| false).unwrap(),
            PrepareStep::Retired
        ));
        assert_eq!(queued_history.lock().unwrap().fresh_starts, queued_starts);

        for offset in (HEAD_FRAMES + 8192..HEAD_FRAMES + 12_288).step_by(256) {
            let at = start + offset as i64;
            actual.render(at, 256, &mut a, &solo_options).unwrap();
            reference.render(at, 256, &mut b, &solo_options).unwrap();
            assert_eq!(bits(&a), bits(&b), "post-retirement PCM at {offset}");
        }
    }

    #[test]
    fn sliced_loop_wrap_prepares_fresh_histories_after_release() {
        let f = Fixture::new();
        let start = 192_000;
        let mut mix = f.mixer(0.7, 1.5, 10.0);
        mix.standard_content(true);
        let mut previous = Vec::new();
        for pass in 0..2 {
            let jobs = mix.preparation_work(start, start);
            assert_eq!(jobs.len(), 4, "four independent jobs on pass {pass}");
            let mut histories = Vec::new();
            for job in jobs {
                let vi = job.vi;
                let history = job.history();
                if pass == 1 {
                    assert!(
                        previous.iter().all(|old| !Arc::ptr_eq(old, &history)),
                        "loop wrap v{vi} must not prepare an installed history"
                    );
                }
                histories.push(history);
                assert!(job.claim());
                let mut task = job.into_task(Evaluator::new(), true);
                let mut yields = 0;
                loop {
                    match task.step(|| false).unwrap() {
                        PrepareStep::Yielded => yields += 1,
                        PrepareStep::Complete(ready) => {
                            assert!(ready);
                            break;
                        }
                        PrepareStep::Retired => panic!("fresh wrap job was retired"),
                    }
                    assert!(yields < 128);
                }
                assert!(yields > 0, "wrap v{vi} actually resumed");
                mix.prepared_voice(start, vi, true);
            }
            mix.use_prepared(start);
            let mut reference = Reference::new(&f);
            reference.compare(&mut mix, start, 0, HEAD_FRAMES + 8192);
            for history in &histories {
                assert_eq!(history.lock().unwrap().fresh_starts, 1);
            }
            previous = histories;
        }
    }

    #[test]
    fn sliced_dynamic_evaluation_matches_offline_after_resume() {
        let f = Fixture::new();
        let speed =
            r#"{"v":1.5,"kf":[{"t":0,"v":1.5,"i":"s0"},{"t":8,"v":1.6,"i":"s1"}],"expr":null}"#;
        for start in [192_000, 196_480] {
            let make = || {
                let (doc, graph) = f.mapped_graph(0.7, speed, 10.0);
                assert!(
                    graph
                        .voices
                        .iter()
                        .all(|voice| voice.constant_rate.is_none())
                );
                Mixer::new(
                    MixFormat::stereo(48_000),
                    doc,
                    graph,
                    Evaluator::new(),
                    f.cache.clone(),
                )
            };
            let mut sliced = make();
            for job in sliced.preparation_work(start, start) {
                let vi = job.vi;
                let history = job.history();
                assert!(job.claim());
                let mut task = job.into_task(Evaluator::new(), false);
                let mut yields = 0;
                loop {
                    let calls_before = history.lock().unwrap().native_calls();
                    let outcome = task.step(|| false).unwrap();
                    let calls_after = history.lock().unwrap().native_calls();
                    // A class-changing produce may process the outgoing
                    // run and output-seek the next one before its boundary.
                    assert!(
                        calls_after - calls_before <= 4,
                        "dynamic v{vi} made too many native calls in one slice"
                    );
                    match outcome {
                        PrepareStep::Yielded => yields += 1,
                        PrepareStep::Complete(ready) => {
                            assert!(ready);
                            break;
                        }
                        PrepareStep::Retired => panic!("dynamic history was not handed off"),
                    }
                    assert!(yields < 128);
                }
                assert!(yields > 0, "dynamic map resumed with its retiming memo");
                sliced.prepared_voice(start, vi, true);
            }
            sliced.use_prepared(start);
            let mut reference = make();
            let options = RenderOptions {
                wait: true,
                ..Default::default()
            };
            let mut a = vec![vec![]; 2];
            let mut b = vec![vec![]; 2];
            for offset in (0..8192).step_by(256) {
                sliced
                    .render(start + offset, 256, &mut a, &options)
                    .unwrap();
                reference
                    .render(start + offset, 256, &mut b, &options)
                    .unwrap();
                for (a, b) in a.iter().zip(&b) {
                    assert_eq!(
                        a.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                        b.iter().map(|x| x.to_bits()).collect::<Vec<_>>(),
                        "dynamic PCM at {start}+{offset}"
                    );
                }
            }
        }
    }

    #[test]
    fn sliced_preparation_matches_unsliced_and_offline_through_head_handoff() {
        fn assert_send<T: Send>() {}
        assert_send::<PrepareTask>();

        let f = Fixture::new();
        let start = 192_000;
        let mut sliced = f.mixer(0.7, 1.5, 10.0);
        let mut unsliced = f.mixer(0.7, 1.5, 10.0);
        sliced.standard_content(true);
        unsliced.standard_content(true);
        let sliced_jobs = sliced.preparation_work(start, start);
        let unsliced_jobs = unsliced.preparation_work(start, start);
        assert_eq!(sliced_jobs.len(), 4);
        assert_eq!(unsliced_jobs.len(), 4);
        let mut saw_two_boundary_slice = false;

        for (job, whole) in sliced_jobs.into_iter().zip(unsliced_jobs) {
            assert_eq!(job.vi, whole.vi);
            let vi = job.vi;
            let finished = job.finished.clone();
            let history = job.history();
            let head = job.head.clone().unwrap();
            assert!(job.claim());
            assert!(whole.claim());
            let mut task = job.into_task(Evaluator::new(), true);
            let mut yields = 0;
            loop {
                let calls_before = history.lock().unwrap().native_calls();
                let outcome = task.step(|| false).unwrap();
                let calls_after = history.lock().unwrap().native_calls();
                assert!(
                    calls_after - calls_before <= 2,
                    "constant-map v{vi} made more than two native calls in one slice"
                );
                assert!(
                    task.last_slice_boundaries <= 2,
                    "v{vi} exceeded the two-boundary slice across a stage transition"
                );
                saw_two_boundary_slice |= task.last_slice_boundaries == 2;
                match outcome {
                    PrepareStep::Yielded => {
                        yields += 1;
                        assert!(!finished.load(Ordering::Acquire));
                        assert!(history.try_lock().is_ok(), "yield releases v{vi}'s history");
                        if vi == 0 {
                            assert!(head.lock().unwrap().is_none());
                            assert!(!head.available.load(Ordering::Acquire));
                        }
                    }
                    PrepareStep::Complete(ready) => {
                        assert!(ready, "sliced v{vi} completed");
                        break;
                    }
                    PrepareStep::Retired => panic!("history was not handed off"),
                }
                assert!(yields < 128, "sliced v{vi} makes bounded progress");
            }
            assert!(yields > 0, "v{vi} exercised a resumed native boundary");
            assert!(finished.load(Ordering::Acquire));
            assert!(whole.run(&mut Evaluator::new(), true, || false).unwrap());
            sliced.prepared_voice(start, vi, true);
            unsliced.prepared_voice(start, vi, true);
        }
        assert!(
            saw_two_boundary_slice,
            "the production two-boundary quota ran"
        );

        let head = sliced.prepared[&start].heads[0].as_ref().unwrap();
        let head_guard = head.lock().unwrap();
        let head = head_guard.as_ref().unwrap();
        assert_eq!(head.pcm.len(), 2);
        assert!(head.pcm.iter().all(|channel| channel.len() == HEAD_FRAMES));
        drop(head_guard);
        sliced.use_prepared(start);
        unsliced.use_prepared(start);
        let mut reference = f.mixer(0.7, 1.5, 10.0);
        let options = RenderOptions {
            wait: true,
            ..Default::default()
        };
        let mut a = vec![vec![]; 2];
        let mut b = vec![vec![]; 2];
        let mut expected = vec![vec![]; 2];
        for offset in (0..HEAD_FRAMES + 8192).step_by(256) {
            let at = start + offset as i64;
            assert!(!sliced.awaiting_history(at, 256));
            sliced.render(at, 256, &mut a, &options).unwrap();
            unsliced.render(at, 256, &mut b, &options).unwrap();
            reference.render(at, 256, &mut expected, &options).unwrap();
            let bits = |pcm: &[Vec<f32>]| {
                pcm.iter()
                    .flat_map(|channel| channel.iter().map(|sample| sample.to_bits()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(bits(&a), bits(&b), "sliced and unsliced PCM at {offset}");
            assert_eq!(
                bits(&a),
                bits(&expected),
                "sliced and offline PCM at {offset}"
            );
        }
    }

    #[test]
    fn a_queued_yield_or_cancellation_never_publishes_a_partial_head() {
        let f = Fixture::new();
        let start = 192_000;
        let mut mix = f.mixer(0.7, 1.5, 10.0);
        mix.standard_content(true);
        let mut jobs = mix.preparation_work(start, start).into_iter();
        let representative = jobs.next().unwrap();
        let dependent = jobs.next().unwrap();
        let head = representative.head.as_ref().unwrap().clone();
        let finished = representative.finished.clone();
        let history = representative.history();
        let gate = dependent.gate.as_ref().unwrap().clone();
        assert!(Arc::ptr_eq(&head, dependent.head.as_ref().unwrap()));
        assert!(representative.claim());
        let mut task = representative.into_task(Evaluator::new(), true);
        assert!(matches!(task.step(|| false).unwrap(), PrepareStep::Yielded));
        assert!(head.lock().unwrap().is_none());
        assert!(!head.available.load(Ordering::Acquire));
        assert!(!gate.load(Ordering::Acquire));
        assert!(!finished.load(Ordering::Acquire));
        assert!(
            history.try_lock().is_ok(),
            "queued continuation owns no mutex"
        );
        drop(task);
        assert!(finished.load(Ordering::Acquire));
        assert!(head.lock().unwrap().is_none());
        assert!(
            gate.load(Ordering::Acquire),
            "cancellation releases dependents"
        );
        assert!(!mix.prepared_ready(&[start]));

        // Cancellation of a dependent behind an installed complete head
        // retires its history without making another worker or the producer
        // claim the same run. The other queued histories still block handoff.
        let (installed, rest) = partly_installed(&f, start);
        let end = start + HEAD_FRAMES as i64;
        let mut rest = rest.into_iter();
        let first = rest.next().unwrap();
        let first_history = first.history();
        let first_finished = first.finished.clone();
        assert!(first.claim());
        let mut task = first.into_task(Evaluator::new(), true);
        assert!(matches!(task.step(|| false).unwrap(), PrepareStep::Yielded));
        assert!(installed.awaiting_history(end, 256));
        assert!(first_history.try_lock().is_ok());
        task.cancel();
        assert!(first_finished.load(Ordering::Acquire));
        assert!(installed.awaiting_history(end, 256));
        drop(task);
        for job in rest {
            assert!(job.claim());
            let mut task = job.into_task(Evaluator::new(), true);
            task.cancel();
        }
        assert!(!installed.awaiting_history(end, 256));
    }

    /// A start installed while three of its four jobs are still queued: the
    /// four voices play the shared head; v1..v3 wait on their histories.
    fn partly_installed(f: &Fixture, start: i64) -> (Mixer, Vec<PrepareVoice>) {
        let mut mix = f.mixer(0.7, 1.5, 10.0);
        mix.standard_content(true);
        let mut jobs = mix.preparation_work(start, start).into_iter();
        let first = jobs.next().unwrap();
        assert!(first.claim());
        assert!(first.run(&mut Evaluator::new(), true, || false).unwrap());
        mix.prepared_voice(start, 0, true);
        let rest: Vec<_> = jobs.collect();
        assert_eq!(rest.len(), 3);
        mix.use_prepared(start);
        assert!(mix.voices[1..].iter().all(|v| v.prefill.is_some()));
        (mix, rest)
    }

    /// The actual pool owns the queued histories behind an installed head.
    /// Boundary preparation cannot take those live histories while they are
    /// unfinished; after the workers release them, a loop wrap gets fresh,
    /// exact histories from a second pool pass.
    #[test]
    fn worker_finished_installed_histories_are_never_rescheduled() {
        use std::sync::atomic::AtomicUsize;

        struct WorkerGate {
            open: std::sync::Mutex<bool>,
            wake: std::sync::Condvar,
            entered: AtomicUsize,
        }
        struct WaitInWorker(Arc<WorkerGate>);
        impl pm_eval::EvalHooks for WaitInWorker {
            fn resolve_content(
                &self,
                _: &pm_eval::Comp<'_>,
                _: &pm_model::Node,
                _: f64,
                _: &mut pm_eval::evaluator::Content,
                _: &mut dyn pm_eval::evaluator::PropEval,
            ) -> pm_eval::js_sem::JsResult<()> {
                self.0.entered.fetch_add(1, Ordering::Release);
                let mut open = self.0.open.lock().unwrap_or_else(|e| e.into_inner());
                while !*open {
                    open = self.0.wake.wait(open).unwrap_or_else(|e| e.into_inner());
                }
                Ok(())
            }
        }
        struct OpenOnDrop(Arc<WorkerGate>);
        impl Drop for OpenOnDrop {
            fn drop(&mut self) {
                *self.0.open.lock().unwrap_or_else(|e| e.into_inner()) = true;
                self.0.wake.notify_all();
            }
        }
        let f = Fixture::new();
        let start = 192_000;
        let mut mix = f.mixer(0.7, 1.5, 10.0);
        mix.standard_content(true);
        let mut jobs = mix.preparation_work(start, start).into_iter();
        let first = jobs.next().unwrap();
        assert!(first.claim());
        assert!(first.run(&mut Evaluator::new(), true, || false).unwrap());
        mix.prepared_voice(start, 0, true);
        assert_eq!(jobs.count(), 3);
        let gate = Arc::new(WorkerGate {
            open: std::sync::Mutex::new(false),
            wake: std::sync::Condvar::new(),
            entered: AtomicUsize::new(0),
        });
        let open_on_drop = OpenOnDrop(gate.clone());
        let mut prep = crate::preparation::Preparation::new();
        prep.evaluators(
            Arc::new({
                let gate = gate.clone();
                move || Evaluator::new().with_hooks(Arc::new(WaitInWorker(gate.clone())))
            }),
            true,
        );
        // A 64-frame callback at 48 kHz leaves 1.333 ms per constraint
        // period. Exercise the same pool DSP with this short device budget.
        let schedule = crate::realtime::AudioSchedule::new(48_000, 64, None);
        prep.set_schedule(schedule.clone());
        prep.schedule(&mut mix, &[start], true);
        assert_eq!(prep.pending().len(), 3);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        while gate.entered.load(Ordering::Acquire) == 0 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert!(
            gate.entered.load(Ordering::Acquire) > 0,
            "pool started a history"
        );
        mix.use_prepared(start);
        let live: Vec<_> = mix
            .voices
            .iter()
            .map(|v| v.stretch.clone().unwrap())
            .collect();
        let mut reference = Reference::new(&f);
        reference.compare(&mut mix, start, 0, HEAD_FRAMES);
        let end = start + HEAD_FRAMES as i64;
        assert!(mix.awaiting_history(end, 256));
        // prepare_boundaries: retain the boundary, then schedule it.
        mix.retain_prepared(&[start]);
        prep.schedule(&mut mix, &[start], false);
        let scheduled: Vec<usize> = (0..4)
            .filter(|vi| prep.pending().contains(&(start, *vi)))
            .collect();
        assert_eq!(
            scheduled,
            [0, 1, 2, 3],
            "one fresh boundary job and three unfinished live jobs"
        );
        assert!(mix.prepared[&start].runs[1..].iter().all(Option::is_none));
        let fresh_first = mix.prepared[&start].runs[0].as_ref().unwrap();
        assert!(live.iter().all(|l| !Arc::ptr_eq(l, fresh_first)));
        for (v, l) in mix.voices.iter().zip(&live) {
            assert!(Arc::ptr_eq(v.stretch.as_ref().unwrap(), l));
        }
        assert!(mix.awaiting_history(end, 256), "no producer takeover");
        drop(open_on_drop);
        while !prep.pending().is_empty() && std::time::Instant::now() < deadline {
            prep.wait(&mut mix, std::time::Duration::from_millis(10));
        }
        assert!(
            prep.pending().is_empty(),
            "installed workers and first refill completed"
        );
        assert!(!mix.awaiting_history(end, 256));
        // The released entry can now submit its remaining fresh histories.
        prep.schedule(&mut mix, &[start], false);
        assert_eq!(prep.pending().len(), 3);
        for vi in 1..4 {
            let run = mix.prepared[&start].runs[vi].as_ref().unwrap();
            assert!(live.iter().all(|l| !Arc::ptr_eq(l, run)), "fresh v{vi}");
        }
        while !prep.pending().is_empty() && std::time::Instant::now() < deadline {
            prep.wait(&mut mix, std::time::Duration::from_millis(10));
        }
        assert!(
            prep.pending().is_empty(),
            "fresh histories completed on the pool"
        );
        // Done follows each schedule guard's restoration, so accepting all
        // reports also makes every worker's final scheduling metrics visible.
        // The artificial gate wait is wall time rather than CPU service.
        let metrics = schedule.metrics();
        eprintln!(
            "A8 64-frame pool: max DSP unit {} ns, max period CPU {} ns, computation overruns {}, constraint overruns {}, throttle sleeps {}, total sleep {} ns, restore failures {}",
            metrics.max_unit_ns,
            metrics.max_period_cpu_ns,
            metrics.computation_overruns,
            metrics.constraint_overruns,
            metrics.throttle_sleeps,
            metrics.total_sleep_ns,
            metrics.restore_failures,
        );
        #[cfg(target_os = "macos")]
        {
            let joined = prep.scheduling.workgroup.load(Ordering::Acquire)
                + prep.scheduling.mach.load(Ordering::Acquire);
            assert_eq!(
                prep.scheduling.jobs_started.load(Ordering::Acquire),
                7,
                "three live jobs and four fresh jobs claimed their histories once"
            );
            assert!(joined >= 7, "every job entered RT for one or more slices");
            assert_eq!(
                joined,
                prep.scheduling.slice_entries.load(Ordering::Acquire),
                "every scheduling attempt entered a real-time method"
            );
            assert_eq!(prep.scheduling.failed.load(Ordering::Acquire), 0);
            assert!(metrics.max_unit_ns > 0, "worker CPU was measured");
            assert_eq!(metrics.constraint_overruns, 0, "64-frame CPU constraint");
            assert_eq!(metrics.restore_failures, 0, "workers restored their policy");
        }
        // The live histories continue exactly through and beyond the handoff.
        for from in (HEAD_FRAMES..HEAD_FRAMES + 16_384).step_by(1024) {
            reference.compare(&mut mix, start, from, from + 1024);
        }
        assert!(mix.prepared_ready(&[start]));
        // The next loop wrap.
        mix.use_prepared(start);
        for (v, l) in mix.voices.iter().zip(&live) {
            assert!(!Arc::ptr_eq(v.stretch.as_ref().unwrap(), l));
            assert!(v.prefill.is_none());
        }
        let mut wrapped = Reference::new(&f);
        wrapped.compare(&mut mix, start, 0, 8192);
        for v in &mix.voices {
            let run = v.stretch.as_ref().unwrap().lock().unwrap();
            assert_eq!(run.fresh_starts, 1, "no producer pre-roll at the wrap");
        }
    }

    /// The entry of an installed start can be dropped (paused preparation
    /// evicts starts, a project invalidates them) while a worker still runs
    /// one of its histories. The job's own completion releases the head's
    /// consumer: it never waits on a result that reaches no entry.
    #[test]
    fn a_dropped_start_entry_never_strands_an_installed_history() {
        let f = Fixture::new();
        let start = 192_000;
        let (mut mix, rest) = partly_installed(&f, start);
        assert!(
            rest.iter()
                .all(|work| work.critical.load(Ordering::Acquire)),
            "installed heads promote every queued independent history"
        );
        let running = rest.iter().position(|j| j.vi == 1).unwrap();
        assert!(rest[running].claim());
        let mut reference = Reference::new(&f);
        reference.compare(&mut mix, start, 0, HEAD_FRAMES);
        mix.invalidate_prepared();
        let end = start + HEAD_FRAMES as i64;
        assert!(mix.awaiting_history(end, 256), "v1 is still running");
        for (i, job) in rest.into_iter().enumerate() {
            if i != running {
                assert!(
                    job.claim(),
                    "queued v{} still belongs to its worker",
                    job.vi
                );
            }
            assert!(job.run(&mut Evaluator::new(), true, || false).unwrap());
            assert_eq!(
                mix.awaiting_history(end, 256),
                i < 2,
                "the installed head follows job completion despite entry eviction"
            );
        }
        reference.compare(&mut mix, start, HEAD_FRAMES, HEAD_FRAMES + 8192);
    }

    /// Paused or boundary preparation of a start that Play installed never
    /// takes a queued job's live history: it prepares fresh ones while the
    /// installed head continues to wait for its original workers.
    #[test]
    fn paused_preparation_never_runs_a_queued_job_on_a_live_history() {
        let f = Fixture::new();
        let start = 192_000;
        let (mut mix, rest) = partly_installed(&f, start);
        assert!(
            rest.iter()
                .all(|work| work.critical.load(Ordering::Acquire)),
            "installed heads promote every queued independent history"
        );
        let live: Vec<_> = mix
            .voices
            .iter()
            .map(|v| v.stretch.clone().unwrap())
            .collect();
        let mut reference = Reference::new(&f);
        reference.compare(&mut mix, start, 0, HEAD_FRAMES);
        prepare(&mut mix, start);
        for (vi, run) in mix.prepared[&start].runs.iter().enumerate() {
            let run = run.as_ref().unwrap();
            assert!(live.iter().all(|l| !Arc::ptr_eq(l, run)), "v{vi}");
        }
        let end = start + HEAD_FRAMES as i64;
        assert!(mix.awaiting_history(end, 256));
        for (i, job) in rest.into_iter().enumerate() {
            assert!(
                job.claim(),
                "paused preparation did not take live v{}",
                job.vi
            );
            assert!(job.run(&mut Evaluator::new(), true, || false).unwrap());
            assert_eq!(mix.awaiting_history(end, 256), i < 2);
        }
        reference.compare(&mut mix, start, HEAD_FRAMES, HEAD_FRAMES + 8192);
    }

    /// A start's own job leaves its opening output in the history: the
    /// producer's first callbacks need no vocoder step, no map point and no
    /// source read.
    #[test]
    fn a_start_job_buffers_its_opening_output() {
        let f = Fixture::new();
        // Cold A8 targets where the vocoder's second output step falls
        // inside the first 512 frames (2 s at 1.6x) and where it does not.
        for (start, speed) in [(96_000, 1.6), (144_000, 1.5), (196_480, 1.5)] {
            let mut mix = f.mixer(0.7, speed, 10.0);
            let job = mix.preparation_work(start, start).remove(0);
            assert!(
                job.head.is_none(),
                "no shared head without standard content"
            );
            let history = job.history();
            assert!(job.run(&mut Evaluator::new(), false, || false).unwrap());
            let lead = lead_frames(48_000);
            assert_eq!(lead, 1024);
            let mut run = history.lock().unwrap();
            let mut out = vec![Vec::new(); 2];
            let mut buffered = vec![vec![]; 2];
            for offset in (0..lead).step_by(256) {
                assert!(
                    run.render(
                        start + offset as i64,
                        256,
                        &mut |_| panic!("no map point inside the lead"),
                        &mut |_, _, _| panic!("no source read inside the lead"),
                        &mut out,
                    )
                    .unwrap()
                );
                for c in 0..2 {
                    buffered[c].extend_from_slice(&out[c]);
                }
            }
            assert!(buffered[0].iter().any(|v| *v != 0.0));
        }
        // The PCM itself is the offline stream: every prepared start is
        // compared bit for bit with a fresh render through the lead and
        // beyond (detached_jobs_use_custom_channel_hooks, the playback
        // sixteen-voice suites).
    }

    /// Nested linear-parent precompositions and a video's embedded audio:
    /// the compiled map reproduces the full evaluator bit for bit at every
    /// level of the stack, so playback stays identical to offline export.
    #[test]
    fn compiled_constant_maps_match_through_precomps_and_embedded_video() {
        let f = Fixture::new();
        let audio = r#"{"id":"A","type":"audio","from":0.1,"dur":9,"on":true,"d":{"asset":"s","gain":0.6,"fadeIn":0.2,"fadeOut":0.3,"trim":0.3,"speed":1.5}}"#;
        let comps = format!(
            r#""X":{{"id":"X","w":64,"h":64,"fps":30,"dur":10,"layers":[{audio}]}},"Y":{{"id":"Y","w":64,"h":64,"fps":30,"dur":10,"layers":[{{"id":"Q","type":"precomp","from":0.05,"dur":9.5,"on":true,"d":{{"comp":"X","trim":0.02,"audioGain":0.9}}}}]}}"#
        );
        let layers = [
            r#"{"id":"P1","type":"precomp","from":0.25,"dur":9.5,"on":true,"d":{"comp":"X","trim":0.1,"audioGain":0.8}}"#,
            r#"{"id":"P2","type":"precomp","from":0.5,"dur":9,"on":true,"d":{"comp":"Y","trim":0.07}}"#,
            r#"{"id":"V","type":"video","from":0.1,"dur":9,"on":true,"d":{"asset":"s","embeddedAudio":true,"audioGain":0.8,"trim":0.2,"speed":1.25,"pitch":"preserve"}}"#,
        ];
        let doc = Arc::new(Document::new(
            Some(
                parse(&format!(
                    r#"{{"id":"P","compId":"C","w":64,"h":64,"fps":30,"dur":10,"layers":[{}],"comps":{{{comps}}}}}"#,
                    layers.join(",")
                ))
                .unwrap(),
            ),
            1,
        ));
        let graph = MixGraph::build(&doc, "C", &mut Evaluator::new(), &f.cache, &|_: &str| {
            Some(f.dir.path().join("tone.wav"))
        })
        .unwrap();
        let ids: Vec<&str> = graph.voices.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids.len(), 3, "voices {ids:?}");
        let mut depths: Vec<usize> = graph.voices.iter().map(|v| v.levels.len()).collect();
        depths.sort();
        assert_eq!(
            depths,
            [1, 2, 3],
            "video, one and two precomposition levels"
        );
        assert!(
            graph
                .voices
                .iter()
                .any(|v| v.kind == crate::graph::VoiceKind::Embedded)
        );
        let comps = graph.resolve(&doc).unwrap();
        let mut ev = Evaluator::new();
        for plan in &graph.voices {
            let fixed = crate::graph::StaticMap::new(plan, &comps, &mut ev, 4.0)
                .unwrap()
                .unwrap_or_else(|| panic!("{} compiles to a static map", plan.id));
            let mut retime = RetimeCache::new();
            for sample in (-48_000..528_000).step_by(16) {
                let t = sample as f64 / 48_000.0;
                let full = plan
                    .evaluate(&mut ev, &comps, &comps[0], t, &mut retime)
                    .unwrap();
                let actual = fixed.evaluate(plan, t);
                assert_eq!(
                    (actual.active, actual.in_span),
                    (full.active, full.in_span),
                    "{} at {t}",
                    plan.id
                );
                for (a, b) in [
                    actual.gain,
                    actual.source_time,
                    actual.local,
                    actual.fade_in,
                    actual.fade_out,
                    actual.audible,
                ]
                .into_iter()
                .zip([
                    full.gain,
                    full.source_time,
                    full.local,
                    full.fade_in,
                    full.fade_out,
                    full.audible,
                ]) {
                    assert_eq!(a.to_bits(), b.to_bits(), "{} at {t}", plan.id);
                }
                let fast = fixed.point(t, 48_000.0);
                assert_eq!(fast.x.to_bits(), (full.source_time * 48_000.0).to_bits());
                assert_eq!(fast.on, full.active && full.in_span);
            }
        }
    }

    #[test]
    fn compiled_constant_maps_keep_the_full_evaluators_exact_arithmetic() {
        let f = Fixture::new();
        for speed in ["1.5", "0.75", r#"{"v":1.6,"kf":[],"expr":""}"#] {
            let (doc, graph) = f.mapped_graph(0.7, speed, 10.0);
            let comps = graph.resolve(&doc).unwrap();
            let mut ev = Evaluator::new();
            let plan = &graph.voices[0];
            let fixed = crate::graph::StaticMap::new(plan, &comps, &mut ev, 4.0)
                .unwrap()
                .unwrap();
            let mut retime = RetimeCache::new();
            for sample in (-48_000..528_000).step_by(16) {
                let t = sample as f64 / 48_000.0;
                let full = plan
                    .evaluate(&mut ev, &comps, &comps[0], t, &mut retime)
                    .unwrap();
                let actual = fixed.evaluate(plan, t);
                assert_eq!((actual.active, actual.in_span), (full.active, full.in_span));
                for (a, b) in [
                    actual.gain,
                    actual.source_time,
                    actual.local,
                    actual.fade_in,
                    actual.fade_out,
                    actual.audible,
                ]
                .into_iter()
                .zip([
                    full.gain,
                    full.source_time,
                    full.local,
                    full.fade_in,
                    full.fade_out,
                    full.audible,
                ]) {
                    assert_eq!(a.to_bits(), b.to_bits());
                }
                let fast = fixed.point(t, 48_000.0);
                assert_eq!(fast.x.to_bits(), (full.source_time * 48_000.0).to_bits());
                assert_eq!(fast.on, full.active && full.in_span);
            }
        }
    }

    #[test]
    fn play_retains_and_replenishes_exact_loop_histories() {
        let f = Fixture::new();
        let mut m = f.mixer(1.0, 1.5, 8.0);
        let loop_start = 4 * 48_000;
        for s in [0, loop_start, 6 * 48_000 - 256] {
            prepare(&mut m, s);
        }
        m.use_prepared(0);
        assert!(m.prepared_ready(&[loop_start, 6 * 48_000 - 256]));
        let mut expected = f.mixer(1.0, 1.5, 8.0);
        let mut a = vec![vec![]; 2];
        let mut b = a.clone();
        let options = RenderOptions {
            wait: true,
            ..Default::default()
        };
        expected.render(loop_start, 4096, &mut b, &options).unwrap();
        for _ in 0..5 {
            m.use_prepared(loop_start);
            for start in (loop_start..loop_start + 4096).step_by(256) {
                m.render(start, 256, &mut a, &options).unwrap();
                let offset = (start - loop_start) as usize;
                for c in 0..2 {
                    assert_eq!(a[c], b[c][offset..offset + 256]);
                }
            }
            for v in &m.voices {
                assert_eq!(v.stretch.as_ref().unwrap().lock().unwrap().fresh_starts, 1);
            }
            prepare(&mut m, loop_start);
        }
    }
    #[test]
    fn level_edits_reuse_histories_but_map_and_span_edits_invalidate() {
        let f = Fixture::new();
        let start = 4 * 48_000;
        let mut m = f.mixer(1.0, 1.5, 8.0);
        prepare(&mut m, start);
        let (doc, graph) = f.graph(0.3, 1.5, 8.0);
        m.set_graph(doc, graph);
        assert!(m.prepared_ready(&[start]));
        m.use_prepared(start);
        let mut reference = f.mixer(0.3, 1.5, 8.0);
        let mut a = vec![vec![]; 2];
        let mut b = a.clone();
        let options = RenderOptions {
            wait: true,
            ..Default::default()
        };
        m.render(start, 512, &mut a, &options).unwrap();
        reference.render(start, 512, &mut b, &options).unwrap();
        assert_eq!(a, b);
        prepare(&mut m, start);
        let (doc, graph) = f.graph(0.3, 1.6, 8.0);
        m.set_graph(doc, graph);
        assert!(!m.prepared_ready(&[start]));
        prepare(&mut m, start);
        let (doc, graph) = f.graph(0.3, 1.6, 7.0);
        m.set_graph(doc, graph);
        assert!(!m.prepared_ready(&[start]));
    }

    #[test]
    fn an_animated_map_is_not_reused_across_a_level_edit() {
        let f = Fixture::new();
        // Gain is static, but the speed channel can depend on other edited
        // properties. VoicePlan::animated describes levels, not this map.
        let speed =
            r#"{"v":1.5,"kf":[{"t":0,"v":1.5,"i":"s0"},{"t":8,"v":1.6,"i":"s1"}],"expr":null}"#;
        let (doc, graph) = f.mapped_graph(1.0, speed, 8.0);
        assert!(!graph.voices[0].animated);
        assert!(graph.voices[0].constant_rate.is_none());
        let mut m = Mixer::new(
            MixFormat::stereo(48_000),
            doc,
            graph,
            Evaluator::new(),
            f.cache.clone(),
        );
        prepare(&mut m, 4 * 48_000);
        let (doc, graph) = f.mapped_graph(0.3, speed, 8.0);
        m.set_graph(doc, graph);
        assert!(!m.prepared_ready(&[4 * 48_000]));
    }
}
