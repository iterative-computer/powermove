//! Pitch-preserving voices (plan section 16: "pitch preserved by default
//! (Signalsmith Stretch, our own binding) ... deterministic pre-roll").
//!
//! A [`StretchRun`] turns a voice's time map into a continuous time
//! stretch of its source. The map is sampled at the mixer's control points
//! (every [`CONTROL`] samples): each control interval reads the source
//! forward, backward, or not at all. An interval is *silent* when the voice
//! is off or out of its span there, when its read rate is below
//! [`HOLD_RATE`] (a held remap: as silent as a held tape), or above
//! [`CUT_RATE`] (a remap jump: a cut). A stretch *run* covers consecutive
//! intervals reading in one direction; it starts where such a stretch of
//! intervals starts (a clip's in-point, after a jump, at a direction
//! change).
//!
//! # Deterministic pre-roll
//!
//! A phase vocoder carries state from everything it has heard, so a run's
//! output at sample n depends on where the run started. Runs therefore
//! start only at points that are a function of the map and of where
//! rendering starts, never of how a render is split into calls:
//!
//! * Continuing a render (the next request starts where the last one
//!   ended, or inside what it produced) continues the run.
//! * A render that starts anywhere else (a seek, a loop wrap, the first
//!   render of an export) starts at the latest run start at or before it,
//!   but no earlier than [`PREROLL`] samples before the [`GRID`] boundary at
//!   or before it, and renders from there (the pre-roll is discarded).
//! * The stretcher is driven in calls of exactly one analysis interval,
//!   aligned to the run's start, so its analysis positions do not depend on
//!   the callers' block sizes either.
//!
//! So playback from a seek equals an offline render from the same sample,
//! and a render is the same whatever block size reads it. A run that has
//! played on from an earlier start differs from a fresh start only in the
//! phases the vocoder chose (the same magnitudes), which is inaudible.
//!
//! Alignment: [`pm_stretch::Stretcher::output_seek`] aligns the first
//! output with the run's first input sample, and each call is fed input up
//! to the map's position one output latency ahead plus the input latency
//! (the library's convention), so the stretched sound stays on the
//! picture: a burst stretched at 0.5× lands within 0.2 ms of its place
//! (`pm-stretch` tests; `tests/stretch.rs` measures clicks through the
//! mixer).

use std::collections::VecDeque;

use pm_stretch::Stretcher;

use crate::error::{AudioError, Result};
use crate::mixer::CONTROL;

/// Grid of fresh-start boundaries (output samples, absolute).
pub const GRID: i64 = 1 << 14;
/// Read rates below this are a hold (silent).
pub const HOLD_RATE: f64 = 1.0 / 64.0;
/// Read rates above this within one control interval are a cut (silent).
pub const CUT_RATE: f64 = 32.0;
/// Constant read rates this close to 1 resample instead of stretching:
/// within about one sample per analysis interval of unity the library does
/// not re-analyse and its output follows the speed (`pm-stretch` tests),
/// and the pitch moves by at most 2.4 cents (at 48 kHz), below hearing.
pub fn unity_band(sample_rate: u32) -> f64 {
    2.0 / (sample_rate as f64 * pm_stretch::DEFAULT_INTERVAL_SECONDS).floor()
}

/// Whether a constant rate is stretched (else resampled).
pub fn stretches_constant(rate: f64, sample_rate: u32) -> bool {
    let r = rate.abs();
    (r - 1.0).abs() >= unity_band(sample_rate) && (HOLD_RATE..=CUT_RATE).contains(&r)
}

/// Fresh starts begin at most this long before their grid boundary: the
/// analysis block plus the output latency, so the vocoder has heard a whole
/// block before the first sample kept.
pub fn preroll(sample_rate: u32) -> i64 {
    let block = (sample_rate as f64 * pm_stretch::DEFAULT_BLOCK_SECONDS) as i64;
    // Output latency of the default preset is under one block.
    let p = 2 * block;
    (p + CONTROL - 1) / CONTROL * CONTROL
}

/// The map at one control point: the source position in project-rate
/// samples and whether the voice sounds there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapPoint {
    pub x: f64,
    pub on: bool,
}

/// What a control interval reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Silent,
    Run(i8),
}

struct Live {
    stretcher: Stretcher,
    dir: i8,
    /// Output sample the run started at (its call grid).
    start: i64,
    /// Next input sample of the run's stream to feed (stream index
    /// `u = dir × x`).
    fed: i64,
}

/// One voice's stretch, shared by the current mix and a retune glide's
/// outgoing mix while their maps agree ([`crate::graph::VoicePlan::map_key`]).
pub struct StretchRun {
    channels: usize,
    rate: u32,
    key: u64,
    /// Map points by control index, contiguous from `points_from`.
    points: VecDeque<MapPoint>,
    points_from: i64,
    /// Output kept: `buf[c][i]` is output sample `buf_start + i`; the run
    /// has produced everything before `pos`.
    buf: Vec<Vec<f32>>,
    buf_start: i64,
    pos: i64,
    live: Option<Live>,
    /// A spare stretcher, reset, from the last run.
    spare: Option<Stretcher>,
    /// Whether every source read so far since the last fresh start was
    /// complete (decoded).
    complete: bool,
    scratch_in: Vec<Vec<f32>>,
    scratch_out: Vec<Vec<f32>>,
    /// Map points evaluated (diagnostics).
    pub evaluated: u64,
    /// Fresh starts (diagnostics).
    pub fresh_starts: u64,
}

/// Evaluates the map at a control index.
pub type MapFn<'a> = dyn FnMut(i64) -> Result<MapPoint> + 'a;
/// Reads project-rate source samples `[lo, lo + n)` into planar buffers;
/// returns whether every sample was decoded.
pub type ReadFn<'a> = dyn FnMut(i64, usize, &mut [Vec<f32>]) -> Result<bool> + 'a;

impl StretchRun {
    pub fn new(channels: usize, rate: u32, key: u64) -> StretchRun {
        StretchRun {
            channels,
            rate,
            key,
            points: VecDeque::new(),
            points_from: 0,
            buf: vec![Vec::new(); channels],
            buf_start: 0,
            pos: i64::MIN,
            live: None,
            spare: None,
            complete: true,
            scratch_in: vec![Vec::new(); channels],
            scratch_out: vec![Vec::new(); channels],
            evaluated: 0,
            fresh_starts: 0,
        }
    }

    pub fn key(&self) -> u64 {
        self.key
    }

    pub fn channels(&self) -> usize {
        self.channels
    }

    /// Offers map points the mixer already evaluated (control indexes from
    /// `first`), so they are not evaluated twice.
    pub fn offer(&mut self, first: i64, points: impl IntoIterator<Item = MapPoint>) {
        for (i, p) in points.into_iter().enumerate() {
            let c = first + i as i64;
            if self.points.is_empty() {
                self.points_from = c;
                self.points.push_back(p);
            } else if c == self.points_from + self.points.len() as i64 {
                self.points.push_back(p);
            }
        }
    }

    fn point(&mut self, c: i64, map: &mut MapFn<'_>) -> Result<MapPoint> {
        let end = self.points_from + self.points.len() as i64;
        if !self.points.is_empty() && c >= self.points_from && c < end {
            return Ok(self.points[(c - self.points_from) as usize]);
        }
        // Extend contiguously (the run reads forward); a point far away
        // restarts the memo.
        if self.points.is_empty() || c < self.points_from || c > end + 4096 {
            self.points.clear();
            self.points_from = c;
            let p = map(c)?;
            self.evaluated += 1;
            self.points.push_back(p);
            return Ok(p);
        }
        for k in end..=c {
            let p = map(k)?;
            self.evaluated += 1;
            self.points.push_back(p);
        }
        Ok(*self.points.back().expect("pushed"))
    }

    fn class(&mut self, k: i64, map: &mut MapFn<'_>) -> Result<Class> {
        let l = self.point(k, map)?;
        let r = self.point(k + 1, map)?;
        if !(l.on || r.on) || !l.x.is_finite() || !r.x.is_finite() {
            return Ok(Class::Silent);
        }
        let d = r.x - l.x;
        let rate = d.abs() / CONTROL as f64;
        if !(HOLD_RATE..=CUT_RATE).contains(&rate) {
            return Ok(Class::Silent);
        }
        Ok(Class::Run(if d > 0.0 { 1 } else { -1 }))
    }

    /// The stream position `u = dir × x` at output sample `n` (linear
    /// between control points).
    fn u_at(&mut self, n: i64, dir: i8, map: &mut MapFn<'_>) -> Result<f64> {
        let k = n.div_euclid(CONTROL);
        let f = n.rem_euclid(CONTROL) as f64 / CONTROL as f64;
        let l = self.point(k, map)?.x;
        let r = self.point(k + 1, map)?.x;
        Ok(dir as f64 * (l + (r - l) * f))
    }

    /// Renders output samples `[a, a + n)` into `out` (planar, one vector per
    /// source channel, resized to `n`). Returns whether every source sample
    /// read was decoded.
    pub fn render(
        &mut self,
        a: i64,
        n: usize,
        map: &mut MapFn<'_>,
        read: &mut ReadFn<'_>,
        out: &mut [Vec<f32>],
    ) -> Result<bool> {
        let end = a + n as i64;
        let continues = self.pos != i64::MIN && a >= self.buf_start && a <= self.pos;
        if !continues {
            self.fresh(a, map, read)?;
            self.fresh_starts += 1;
        }
        while self.pos < end {
            self.produce(end, map, read)?;
        }
        for (o, b) in out.iter_mut().zip(&self.buf) {
            o.clear();
            let from = (a - self.buf_start) as usize;
            o.extend_from_slice(&b[from..from + n]);
        }
        // Keep this render's range (a glide's outgoing mix renders it
        // again) and nothing before it.
        let drop = (a - self.buf_start) as usize;
        if drop > 0 {
            for b in &mut self.buf {
                b.drain(..drop);
            }
            self.buf_start = a;
        }
        // Map points the run no longer reads.
        let keep = self.buf_start.div_euclid(CONTROL) - 2;
        while self.points_from < keep && self.points.len() > 2 {
            self.points.pop_front();
            self.points_from += 1;
        }
        Ok(std::mem::replace(&mut self.complete, true))
    }

    /// Starts rendering at `a` without a usable history.
    fn fresh(&mut self, a: i64, map: &mut MapFn<'_>, read: &mut ReadFn<'_>) -> Result<()> {
        self.end_live();
        let g = a.div_euclid(GRID) * GRID - preroll(self.rate);
        let k0 = g.div_euclid(CONTROL);
        let ka = a.div_euclid(CONTROL);
        // The latest run start in [k0, ka]; a run already going at k0
        // starts there.
        let mut start = None;
        for k in k0..=ka {
            let class = self.class(k, map)?;
            match class {
                Class::Silent => start = None,
                Class::Run(dir) => {
                    if k == k0 || self.class(k - 1, map)? != class {
                        start = Some((k, dir));
                    }
                }
            }
        }
        for b in &mut self.buf {
            b.clear();
        }
        match start {
            Some((k, dir)) => {
                let s = k * CONTROL;
                self.buf_start = s;
                self.pos = s;
                self.begin(s, dir, map, read)?;
            }
            None => {
                self.buf_start = a;
                self.pos = a;
            }
        }
        Ok(())
    }

    fn end_live(&mut self) {
        if let Some(live) = self.live.take() {
            self.spare = Some(live.stretcher);
        }
    }

    /// Starts a run at output sample `s` (a control point) reading in `dir`.
    fn begin(&mut self, s: i64, dir: i8, map: &mut MapFn<'_>, read: &mut ReadFn<'_>) -> Result<()> {
        self.end_live();
        let mut stretcher = match self.spare.take() {
            Some(mut st) => {
                st.reset().map_err(stretch_error)?;
                st
            }
            None => Stretcher::new(self.channels, self.rate).map_err(stretch_error)?,
        };
        let k = s.div_euclid(CONTROL);
        let l = self.point(k, map)?.x;
        let r = self.point(k + 1, map)?.x;
        let rate = ((r - l).abs() / CONTROL as f64).clamp(HOLD_RATE, CUT_RATE);
        let u0 = (dir as f64 * l).round() as i64;
        let len = stretcher.output_seek_length(rate).max(1);
        self.stream(u0, len, dir, read)?;
        {
            let views: Vec<&[f32]> = self.scratch_in.iter().map(|c| &c[..len]).collect();
            stretcher.output_seek(&views).map_err(stretch_error)?;
        }
        self.live = Some(Live {
            stretcher,
            dir,
            start: s,
            fed: u0 + len as i64,
        });
        Ok(())
    }

    /// Reads stream samples `[u0, u0 + len)` (stream index `u = dir × x`)
    /// into `scratch_in`.
    fn stream(&mut self, u0: i64, len: usize, dir: i8, read: &mut ReadFn<'_>) -> Result<()> {
        for c in &mut self.scratch_in {
            c.clear();
            c.resize(len, 0.0);
        }
        if len == 0 {
            return Ok(());
        }
        let lo = if dir > 0 { u0 } else { -(u0 + len as i64) + 1 };
        self.complete &= read(lo, len, &mut self.scratch_in)?;
        if dir < 0 {
            for c in &mut self.scratch_in {
                c[..len].reverse();
            }
        }
        Ok(())
    }

    /// The first control index in `[from, to]` whose interval does not
    /// continue a run in `dir`.
    fn run_end(&mut self, from: i64, to: i64, dir: i8, map: &mut MapFn<'_>) -> Result<Option<i64>> {
        for k in from..=to {
            if self.class(k, map)? != Class::Run(dir) {
                return Ok(Some(k));
            }
        }
        Ok(None)
    }

    /// Produces output from `pos` up to the next call boundary, class
    /// change, or (when dormant) control point.
    fn produce(&mut self, end: i64, map: &mut MapFn<'_>, read: &mut ReadFn<'_>) -> Result<()> {
        let Some(live) = self.live.as_ref() else {
            let k = self.pos.div_euclid(CONTROL);
            if self.pos.rem_euclid(CONTROL) == 0
                && let Class::Run(dir) = self.class(k, map)?
            {
                return self.begin(self.pos, dir, map, read);
            }
            // Silent up to the next control point (or, over a long silence,
            // straight to where the render ends).
            let mut stop = (k + 1) * CONTROL;
            while stop < end && self.class(stop.div_euclid(CONTROL), map)? == Class::Silent {
                stop += CONTROL;
            }
            let n = (stop - self.pos) as usize;
            for b in &mut self.buf {
                b.resize(b.len() + n, 0.0);
            }
            self.pos = stop;
            return Ok(());
        };
        let (dir, start) = (live.dir, live.start);
        let interval = live.stretcher.interval_samples() as i64;
        let (in_lat, out_lat) = (
            live.stretcher.input_latency() as i64,
            live.stretcher.output_latency() as i64,
        );
        let call_end = start + ((self.pos - start) / interval + 1) * interval;
        // A class change inside the call ends the run there.
        let k_from = self.pos.div_euclid(CONTROL) + 1;
        let k_to = (call_end - 1).div_euclid(CONTROL);
        let change = if k_from <= k_to {
            self.run_end(k_from, k_to, dir, map)?
        } else {
            None
        };
        let stop = change.map_or(call_end, |k| k * CONTROL);
        let m = (stop - self.pos) as usize;
        // Input up to the map one output latency ahead, plus the input
        // latency; past the run's end, extrapolated along its last interval.
        let look = stop + out_lat;
        let k_look = look.div_euclid(CONTROL);
        let run_end = match change {
            Some(k) => Some(k),
            None => self.run_end(k_to.max(k_from - 1) + 1, k_look, dir, map)?,
        };
        let target = match run_end {
            Some(k) if k * CONTROL <= look => {
                let e = k * CONTROL;
                let ue = self.u_at(e, dir, map)?;
                let slope = (ue - self.u_at(e - CONTROL, dir, map)?) / CONTROL as f64;
                ue + (look - e) as f64 * slope
            }
            _ => self.u_at(look, dir, map)?,
        };
        let fed = self.live.as_ref().expect("live").fed;
        let to = (target.round() as i64 + in_lat).max(fed);
        let len = (to - fed) as usize;
        self.stream(fed, len, dir, read)?;
        for c in &mut self.scratch_out {
            c.clear();
            c.resize(m, 0.0);
        }
        {
            let live = self.live.as_mut().expect("live");
            let input: Vec<&[f32]> = self.scratch_in.iter().map(|c| &c[..len]).collect();
            let mut output: Vec<&mut [f32]> =
                self.scratch_out.iter_mut().map(|c| &mut c[..m]).collect();
            live.stretcher
                .process(if len == 0 { &[] } else { &input }, &mut output)
                .map_err(stretch_error)?;
            live.fed = to;
        }
        for (b, o) in self.buf.iter_mut().zip(&self.scratch_out) {
            b.extend_from_slice(&o[..m]);
        }
        self.pos = stop;
        if let Some(k) = change {
            self.end_live();
            if let Class::Run(next) = self.class(k, map)? {
                self.begin(stop, next, map, read)?;
            }
        }
        Ok(())
    }
}

fn stretch_error(e: pm_stretch::StretchError) -> AudioError {
    AudioError::Device(e.to_string())
}
