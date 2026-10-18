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

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

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
}

struct Prepared {
    runs: Vec<Option<Arc<std::sync::Mutex<StretchRun>>>>,
    ready: bool,
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
        self.prepared.clear();
        self.ev = ev;
        self.retime.clear();
        self.reads.clear();
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

    /// Prepare one paused start without touching the live vocoder histories,
    /// meters, transport, or device. Missing input never blocks this worker.
    pub(crate) fn prepare_start(&mut self, start: i64) -> bool {
        if self.prepared.get(&start).is_some_and(|p| p.ready) {
            return true;
        }
        let mut prepared = self.prepared.remove(&start).unwrap_or_else(|| Prepared {
            runs: self
                .voices
                .iter()
                .map(|v| {
                    v.stretch.as_ref().map(|run| {
                        let run = run.lock().unwrap_or_else(|e| e.into_inner());
                        Arc::new(std::sync::Mutex::new(StretchRun::new(
                            run.channels(),
                            self.format.sample_rate,
                            run.key(),
                        )))
                    })
                })
                .collect(),
            ready: false,
        });
        for (v, run) in self.voices.iter_mut().zip(&mut prepared.runs) {
            std::mem::swap(&mut v.stretch, run);
        }
        let mut out = vec![Vec::new(); self.format.channels()];
        // Outgoing glides belong to the previous audible run. Preparing
        // a new start must neither advance them nor touch their vocoders.
        let result = self.render_current(
            start,
            crate::ring::CHUNK,
            &mut out,
            &RenderOptions::default(),
            None,
        );
        for (v, run) in self.voices.iter_mut().zip(&mut prepared.runs) {
            std::mem::swap(&mut v.stretch, run);
        }
        prepared.ready = result.is_ok_and(|r| r.starved == 0 && r.errors.is_empty());
        let ready = prepared.ready;
        if self.prepared.len() >= 3 {
            self.prepared.clear();
        }
        self.prepared.insert(start, prepared);
        ready
    }

    /// Every start in `starts` holds a ready prepared history.
    pub(crate) fn prepared_ready(&self, starts: &[i64]) -> bool {
        starts
            .iter()
            .all(|s| self.prepared.get(s).is_some_and(|p| p.ready))
    }

    pub(crate) fn retain_prepared(&mut self, starts: &[i64]) {
        self.prepared.retain(|start, _| starts.contains(start));
    }

    /// Install the exact pre-rolled history for this paused start, if ready.
    pub(crate) fn use_prepared(&mut self, start: i64) {
        if let Some(prepared) = self.prepared.remove(&start).filter(|p| p.ready) {
            for (v, run) in self.voices.iter_mut().zip(prepared.runs) {
                v.stretch = run;
            }
        }
        self.prepared.clear();
    }

    /// Whether a retune glide is in progress.
    pub fn is_gliding(&self) -> bool {
        !self.outgoing.is_empty()
    }

    fn install(&mut self, doc: Arc<Document>, graph: Arc<MixGraph>) {
        self.prepared.clear();
        let p = self.format.sample_rate;
        // Stretches in progress carry over to voices whose map did not
        // change (a fader drag during playback must not restart them).
        let mut runs: HashMap<(String, u64), Arc<std::sync::Mutex<StretchRun>>> = HashMap::new();
        let previous = self.graph.voices.iter().zip(&self.voices).chain(
            self.outgoing
                .last()
                .map(|g| g.graph.voices.iter().zip(&g.voices))
                .into_iter()
                .flatten(),
        );
        for (plan, voice) in previous {
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
                let pt = plan.evaluate(&mut self.ev, comps, open, t, &mut self.retime)?;
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
        let complete = run.render(a, n, &mut map, &mut read, &mut self.voice_out);
        self.src = src;
        let complete = complete?;
        if !complete && options.drop_starved {
            self.voice_dropped_frames += n as u64;
            // Keep valid vocoder history through short dropped spans. The
            // run bounds catch-up work itself; missing PCM never enters DSP.
            run.commit_drop(a);
        }
        drop(run);
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
        let p = self.format.sample_rate as f64;
        let graph = self.graph.clone();
        let doc = self.doc.clone();
        let end = start + frames as i64;
        let mut out = Vec::new();
        // Time-mapped voices: (index, first and last sample heard here).
        let mut mapped: Vec<(usize, i64, i64)> = Vec::new();
        for (vi, plan) in graph.voices.iter().enumerate() {
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
                Reader::Free => kernel::free_bank(rate / p).taps as i64,
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
        let r = plan.evaluate(ev, comps, open, c as f64 / p, retime)?;
        if let Some(l) = prev
            && (on(&l) || on(&r))
            && c > window.0
            && c - CONTROL < window.1
        {
            let (x0, x1) = (l.source_time * rate, r.source_time * rate);
            if x0.is_finite() && x1.is_finite() {
                // The filter `render_voice` reads this interval with.
                let step = (r.source_time - l.source_time).abs() * rate / CONTROL as f64;
                let bank = kernel::free_bank(step.max(1.0));
                // A stretch reads ahead of its position by its latencies
                // (under two analysis blocks of output), in either
                // direction, through the project-rate reader.
                let pad = stretch_block.map_or(0, |b| {
                    (2.0 * b * step.max(rate / p)).ceil() as i64 + kernel::TAPS as i64
                });
                let lo = x0.min(x1).floor() as i64 - bank.before() as i64 - pad;
                let hi = x0.max(x1).floor() as i64 + (bank.taps - bank.before()) as i64 + pad;
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
            for i in 0..n {
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
            for i in 0..n {
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
