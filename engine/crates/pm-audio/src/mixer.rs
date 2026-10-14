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

use std::collections::HashSet;
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
use crate::source::SourceCache;

/// Samples between automation control points (3 kHz at 48 kHz).
pub const CONTROL: i64 = 16;

/// A live edit during playback (a fader drag, a mute, a monitor solo) glides
/// from what played into the new mix over this long instead of stepping, as
/// today's engine does (`RETUNE_GLIDE` in `legacy/core/audio.ts`): hard steps
/// on a sustained sound are audible as clicks and zipper noise.
pub const RETUNE_GLIDE_MS: u32 = 12;

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
    /// Strips heard (the mixer's monitor solo; never part of an export).
    /// `None` or empty: every strip.
    pub monitor_solo: Option<HashSet<String>>,
    /// Update the level taps.
    pub meters: bool,
}

/// What a render met.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RenderReport {
    /// Voices that sounded somewhere in the range.
    pub voices: usize,
    /// Voices that had source blocks missing (playback only).
    pub starved: usize,
    /// Errors decoding or evaluating, one per voice.
    pub errors: Vec<(String, String)>,
}

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
            voice_out: vec![],
            strips: vec![],
            kernel: vec![],
            points: vec![],
            outgoing: vec![],
            glide_len: 0,
            glide_acc: vec![],
            glide_tmp: vec![],
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
        self.ev = ev;
        self.retime.clear();
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

    /// Whether a retune glide is in progress.
    pub fn is_gliding(&self) -> bool {
        !self.outgoing.is_empty()
    }

    fn install(&mut self, doc: Arc<Document>, graph: Arc<MixGraph>) {
        let p = self.format.sample_rate;
        self.voices = graph
            .voices
            .iter()
            .map(|v| Voice {
                reader: reader_for(v, p),
                matrix: layout::matrix(&v.source.info.speakers, &self.format.speakers),
                strip: graph.strips.iter().position(|s| *s == v.strip).unwrap_or(0),
            })
            .collect();
        if self.taps.len() != graph.strips.len() {
            self.taps = (0..graph.strips.len())
                .map(|_| Tap::new(self.format.channels()))
                .collect();
        }
        self.master_gain = graph.master as f32;
        self.retime.clear();
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
        let report = self.render_current(
            start,
            frames,
            out,
            options,
            options.monitor_solo.as_ref(),
            options.meters,
        )?;
        if !self.outgoing.is_empty() && frames > 0 {
            self.render_glides(start, frames, out, options);
        }
        if options.meters {
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
            if self
                .render_current(start, frames, target, options, solo.as_ref(), false)
                .is_err()
            {
                for c in target.iter_mut() {
                    c.clear();
                    c.resize(frames, 0.0);
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
            g.done += frames;
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

    /// Mixes the current state (no glides, no master tap).
    fn render_current(
        &mut self,
        start: i64,
        frames: usize,
        out: &mut [Vec<f32>],
        options: &RenderOptions,
        monitor_solo: Option<&HashSet<String>>,
        meters: bool,
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
        for (vi, plan) in graph.voices.iter().enumerate() {
            match self.render_voice(vi, plan, &comps, &open, start, frames, options) {
                Ok(Some(starved)) => {
                    report.voices += 1;
                    report.starved += starved as usize;
                }
                Ok(None) => {}
                Err(e) => report.errors.push((plan.id.clone(), e.to_string())),
            }
        }
        let solo = monitor_solo.filter(|s| !s.is_empty());
        for (si, strip) in self.strips.iter().enumerate() {
            if meters {
                self.taps[si].push(strip, frames);
            }
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
        let first = sample_at(plan.window.0, p);
        let last = sample_at(plan.window.1, p);
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
                complete &= self
                    .cache
                    .read(&source, a - shift, n, &mut views, options.wait)?;
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
                complete &= self
                    .cache
                    .read(&source, lo, span, &mut views, options.wait)?;
                for i in 0..n {
                    if self.gains[i] == 0.0 {
                        continue;
                    }
                    let u = unit(a + i as i64);
                    let (si, ph) = (u.div_euclid(*phases) as i64, u.rem_euclid(*phases) as usize);
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
                    complete &= self
                        .cache
                        .read(&source, lo, span, &mut views, options.wait)?;
                    self.kernel.resize(bank.taps, 0.0);
                    for (j, &pos) in seg.iter().enumerate() {
                        if self.gains[i + j] == 0.0 {
                            continue;
                        }
                        let fl = pos.floor();
                        bank.free_kernel(pos - fl, &mut self.kernel);
                        let at = (fl as i64 - bank.before() as i64 - lo) as usize;
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

    /// Queues the source blocks a playback of `[start, start + frames)`
    /// will need, for the decode workers.
    pub fn prefetch(&mut self, start: i64, frames: usize) {
        let p = self.format.sample_rate as f64;
        let graph = self.graph.clone();
        for plan in &graph.voices {
            let first = sample_at(plan.window.0, p);
            let last = sample_at(plan.window.1, p);
            let (a, b) = (start.max(first), (start + frames as i64).min(last));
            if b <= a {
                continue;
            }
            let rate = plan.source.sample_rate() as f64;
            let span = match plan.offset {
                Some(o) => {
                    let s0 = ((a as f64 / p - o) * rate).floor() as i64 - kernel::TAPS as i64;
                    let s1 = ((b as f64 / p - o) * rate).ceil() as i64 + kernel::TAPS as i64;
                    Some((s0, s1))
                }
                None => None,
            };
            if let Some((s0, s1)) = span {
                let lo = (s0.max(0) as u64) / crate::source::BLOCK as u64;
                let hi = (s1.max(0) as u64) / crate::source::BLOCK as u64;
                self.cache.prefetch(&plan.source, lo..=hi);
            }
        }
    }

    /// Whether the blocks for `[start, start + frames)` of every linear voice
    /// are decoded (playback waits for this before it starts).
    pub fn is_ready(&self, start: i64, frames: usize) -> bool {
        let p = self.format.sample_rate as f64;
        self.graph.voices.iter().all(|plan| {
            let first = sample_at(plan.window.0, p);
            let last = sample_at(plan.window.1, p);
            let (a, b) = (start.max(first), (start + frames as i64).min(last));
            if b <= a {
                return true;
            }
            let Some(o) = plan.offset else { return true };
            let rate = plan.source.sample_rate() as f64;
            let s0 = ((a as f64 / p - o) * rate).floor() as i64 - kernel::TAPS as i64;
            let s1 = ((b as f64 / p - o) * rate).ceil() as i64 + kernel::TAPS as i64;
            self.cache.is_ready(&plan.source, s0, (s1 - s0) as usize)
        })
    }
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

/// The first project sample at or after a time (`ceil(t × rate)`).
pub fn sample_at(t: f64, rate: f64) -> i64 {
    if t == f64::INFINITY {
        return i64::MAX / 4;
    }
    if t == f64::NEG_INFINITY {
        return i64::MIN / 4;
    }
    (t * rate).ceil() as i64
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
