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

// Called only on a scheduled audio thread. It does not panic or change the
// stretcher's DSP state.
extern "C" fn stretch_checkpoint() {
    crate::realtime::checkpoint_dsp_step();
}

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

/// The most output a committed drop may leave for a run to catch up on
/// before it starts fresh instead: the most a fresh start ever renders
/// before its first kept sample (one [`GRID`] step plus the pre-roll).
///
/// The unit is render work, in project-rate output samples, not device
/// time: catching up analyses every skipped interval with its real input,
/// so its cost is the output distance it renders, whatever the device or
/// shuttle speed. Within this bound, re-entry continues the vocoder's phase
/// history exactly (it equals a render that never dropped) at no more cost
/// than a fresh start's worst case; past it, a fresh start's pre-roll is
/// cheaper. 0.58 s at 48 kHz (`measure_gap_catch_up`: catch-up and fresh
/// start cost about the same at 0.5 s, and catch-up grows linearly after).
pub fn catch_up_limit(sample_rate: u32) -> i64 {
    GRID + preroll(sample_rate)
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
    #[cfg(test)]
    test_preroll: Option<i64>,
    /// A missing read or detached preparation CPU yield paused the render of
    /// output samples `[a, end)`. Keep the valid history so the next call can
    /// continue that render: a retry of it, or
    /// a request starting inside it or where it ended (the continuation
    /// rule, for a caller that moves on past the miss). Any other request
    /// (a seek, a loop wrap, a direction change) starts fresh by the
    /// pre-roll rules, so it never walks the run over the gap.
    pending: Option<(i64, i64)>,
    /// First committed silent request since this run stopped producing.
    dropped_from: Option<i64>,
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

/// A detached preparation slice never reports a CPU yield as missing media.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrepareOutcome {
    Complete,
    MissingInput,
    Yielded,
}

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
            #[cfg(test)]
            test_preroll: None,
            pending: None,
            dropped_from: None,
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

    pub(crate) fn can_continue_at(&self, at: i64) -> bool {
        self.pos != i64::MIN && at >= self.buf_start && at <= self.pos
    }

    /// A voice prepared off-thread joins a running start. Catch up with real
    /// input through the deliberately silent prefix, preserving the original
    /// phase history (the offline render from `start`), never a new seek.
    pub(crate) fn prepare_join(
        &mut self,
        start: i64,
        at: i64,
        map: &mut MapFn<'_>,
        read: &mut ReadFn<'_>,
        out: &mut [Vec<f32>],
    ) -> Result<bool> {
        let outcome = self.prepare_join_budgeted(start, at, map, read, out, &mut || false)?;
        Ok(match outcome {
            PrepareOutcome::Complete => true,
            PrepareOutcome::MissingInput => false,
            PrepareOutcome::Yielded => unreachable!("unsliced preparation cannot yield"),
        })
    }

    /// The same join, pausing only after a complete native processing call.
    /// The caller keeps ownership of this history across `Yielded` slices.
    pub(crate) fn prepare_join_budgeted(
        &mut self,
        start: i64,
        at: i64,
        map: &mut MapFn<'_>,
        read: &mut ReadFn<'_>,
        out: &mut [Vec<f32>],
        should_yield: &mut dyn FnMut() -> bool,
    ) -> Result<PrepareOutcome> {
        if self.pos < start + crate::ring::CHUNK as i64 {
            match self.render_budgeted(start, crate::ring::CHUNK, map, read, out, should_yield)? {
                PrepareOutcome::Complete => {}
                other => return Ok(other),
            }
        }
        if at > start {
            if !self.can_continue_at(at) {
                self.pending = Some((self.buf_start, at));
                self.dropped_from.get_or_insert(start);
            }
            self.render_budgeted(at, crate::ring::CHUNK, map, read, out, should_yield)
        } else {
            Ok(PrepareOutcome::Complete)
        }
    }

    /// Forgets a starved request, so the next render is judged by the
    /// pre-roll rules alone (playback calls this when it seeks or starts a
    /// run: what it renders next is a new request, not a retry).
    pub fn abandon_retry(&mut self) {
        self.pending = None;
        self.dropped_from = None;
    }

    /// The caller advanced past an incomplete request as intentional silence.
    /// Phase history carries over the gap while catching up stays within
    /// [`catch_up_limit`]; ordinary retries remain unbounded in output
    /// distance (their device-time budget belongs to playback).
    pub fn commit_drop(&mut self, start: i64) {
        self.dropped_from.get_or_insert(start);
        if self.live.is_none() {
            // The run never started (its first input is missing), so there
            // is no phase history to keep: the next request starts fresh at
            // its own sample, and a voice that comes back after a long drop
            // equals a render started where it comes back.
            self.pos = i64::MIN;
        }
    }

    /// The output range of a starved render the next request may continue.
    pub fn pending_retry(&self) -> Option<(i64, i64)> {
        self.pending
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
            let p = {
                let _timing = crate::profile::timer(crate::profile::Kind::Map);
                map(c)?
            };
            self.evaluated += 1;
            self.points.push_back(p);
            return Ok(p);
        }
        for k in end..=c {
            let p = {
                let _timing = crate::profile::timer(crate::profile::Kind::Map);
                map(k)?
            };
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
        let outcome = self.render_budgeted(a, n, map, read, out, &mut || false)?;
        Ok(match outcome {
            PrepareOutcome::Complete => true,
            PrepareOutcome::MissingInput => false,
            PrepareOutcome::Yielded => unreachable!("unsliced render cannot yield"),
        })
    }

    /// Renders with an optional CPU fairness boundary after a whole native
    /// call has committed its output and advanced the run. A yield retains
    /// all DSP state and never means that source input is missing.
    pub(crate) fn render_budgeted(
        &mut self,
        a: i64,
        n: usize,
        map: &mut MapFn<'_>,
        read: &mut ReadFn<'_>,
        out: &mut [Vec<f32>],
        should_yield: &mut dyn FnMut() -> bool,
    ) -> Result<PrepareOutcome> {
        let end = a + n as i64;
        for o in out.iter_mut() {
            o.clear();
            o.resize(n, 0.0);
        }
        let resumes = self
            .pending
            .take()
            .is_some_and(|(from, to)| (from..=to).contains(&a));
        // Retain uninterrupted phase history through a committed drop while
        // catching up costs no more than a fresh start can, but never replay
        // an unbounded outage on the audio render thread.
        let resumes = resumes
            && (self.dropped_from.is_none()
                || end.saturating_sub(self.pos) <= catch_up_limit(self.rate) + n as i64);
        let continues = self.pos != i64::MIN && a >= self.buf_start && (a <= self.pos || resumes);
        if !continues {
            self.dropped_from = None;
            self.fresh_starts += 1;
            if !self.fresh(a, map, read)? {
                self.pending = Some((a, end));
                return Ok(PrepareOutcome::MissingInput);
            }
            if self.pos < end && should_yield() {
                self.pending = Some((a, end));
                return Ok(PrepareOutcome::Yielded);
            }
        }
        while self.pos < end {
            if !self.produce(end, map, read)? {
                self.pending = Some((a, end));
                return Ok(PrepareOutcome::MissingInput);
            }
            if self.pos < end && should_yield() {
                self.pending = Some((a, end));
                return Ok(PrepareOutcome::Yielded);
            }
        }
        for (o, b) in out.iter_mut().zip(&self.buf) {
            o.clear();
            let from = (a - self.buf_start) as usize;
            o.extend_from_slice(&b[from..from + n]);
        }
        self.dropped_from = None;
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
        Ok(PrepareOutcome::Complete)
    }

    /// Starts rendering at `a` without a usable history.
    fn fresh(&mut self, a: i64, map: &mut MapFn<'_>, read: &mut ReadFn<'_>) -> Result<bool> {
        self.end_live();
        let pre = preroll(self.rate);
        #[cfg(test)]
        let pre = self.test_preroll.unwrap_or(pre);
        let g = a.div_euclid(GRID) * GRID - pre;
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
                return self.begin(s, dir, map, read);
            }
            None => {
                self.buf_start = a;
                self.pos = a;
            }
        }
        Ok(true)
    }

    fn end_live(&mut self) {
        if let Some(live) = self.live.take() {
            self.spare = Some(live.stretcher);
        }
    }

    /// Starts a run at output sample `s` (a control point) reading in `dir`.
    fn begin(
        &mut self,
        s: i64,
        dir: i8,
        map: &mut MapFn<'_>,
        read: &mut ReadFn<'_>,
    ) -> Result<bool> {
        self.end_live();
        crate::realtime::checkpoint_phase("stretch-map");
        let k = s.div_euclid(CONTROL);
        let l = self.point(k, map)?.x;
        let r = self.point(k + 1, map)?.x;
        let rate = ((r - l).abs() / CONTROL as f64).clamp(HOLD_RATE, CUT_RATE);
        let u0 = (dir as f64 * l).round() as i64;
        // A canceled map must not construct or reset a stretcher.
        crate::realtime::checkpoint_phase_reserve("stretch-init", 1_000_000);
        let mut stretcher = {
            let _timing = crate::profile::timer(crate::profile::Kind::Dsp);
            let _scope = crate::profile::timer(crate::profile::Kind::DspInit);
            match self.spare.take() {
                Some(mut st) => {
                    st.reset().map_err(stretch_error)?;
                    st
                }
                None => Stretcher::new(self.channels, self.rate).map_err(stretch_error)?,
            }
        };
        crate::realtime::checkpoint_phase("stretch-map");
        let len = stretcher.output_seek_length(rate).max(1);
        crate::realtime::checkpoint_phase("stretch-read");
        if !self.stream(u0, len, dir, read)? {
            self.spare = Some(stretcher);
            return Ok(false);
        }
        {
            let views: Vec<&[f32]> = self.scratch_in.iter().map(|c| &c[..len]).collect();
            crate::realtime::checkpoint_phase("stretch-output-seek");
            let _timing = crate::profile::timer(crate::profile::Kind::Dsp);
            let _scope = crate::profile::timer(crate::profile::Kind::DspSeek);
            if crate::realtime::has_active_guard() {
                stretcher
                    .output_seek_with_checkpoint(&views, stretch_checkpoint)
                    .map_err(stretch_error)?;
            } else {
                stretcher.output_seek(&views).map_err(stretch_error)?;
            }
        }
        crate::realtime::checkpoint_phase("stretch-map");
        self.live = Some(Live {
            stretcher,
            dir,
            start: s,
            fed: u0 + len as i64,
        });
        Ok(true)
    }

    /// Reads stream samples `[u0, u0 + len)` (stream index `u = dir × x`)
    /// into `scratch_in`.
    fn stream(&mut self, u0: i64, len: usize, dir: i8, read: &mut ReadFn<'_>) -> Result<bool> {
        for c in &mut self.scratch_in {
            c.clear();
            c.resize(len, 0.0);
        }
        if len == 0 {
            return Ok(true);
        }
        let lo = if dir > 0 { u0 } else { -(u0 + len as i64) + 1 };
        let complete = {
            let _timing = crate::profile::timer(crate::profile::Kind::Read);
            read(lo, len, &mut self.scratch_in)?
        };
        if !complete {
            return Ok(false);
        }
        if dir < 0 {
            for c in &mut self.scratch_in {
                c[..len].reverse();
            }
        }
        Ok(true)
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
    fn produce(&mut self, end: i64, map: &mut MapFn<'_>, read: &mut ReadFn<'_>) -> Result<bool> {
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
            return Ok(true);
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
        // Never feed zero-filled, undecoded input to the vocoder: it
        // changes every later analysis even after the cache catches up.
        crate::realtime::checkpoint_phase("stretch-read");
        if !self.stream(fed, len, dir, read)? {
            return Ok(false);
        }
        for c in &mut self.scratch_out {
            c.clear();
            c.resize(m, 0.0);
        }
        {
            let live = self.live.as_mut().expect("live");
            let input: Vec<&[f32]> = self.scratch_in.iter().map(|c| &c[..len]).collect();
            let mut output: Vec<&mut [f32]> =
                self.scratch_out.iter_mut().map(|c| &mut c[..m]).collect();
            crate::realtime::checkpoint_phase("stretch-process");
            let _timing = crate::profile::timer(crate::profile::Kind::Dsp);
            let _scope = crate::profile::timer(crate::profile::Kind::DspProcess);
            let input: &[&[f32]] = if len == 0 { &[] } else { &input };
            if crate::realtime::has_active_guard() {
                live.stretcher
                    .process_with_checkpoint(input, &mut output, stretch_checkpoint)
                    .map_err(stretch_error)?;
            } else {
                live.stretcher
                    .process(input, &mut output)
                    .map_err(stretch_error)?;
            }
            live.fed = to;
        }
        crate::realtime::checkpoint_phase("stretch-map");
        for (b, o) in self.buf.iter_mut().zip(&self.scratch_out) {
            b.extend_from_slice(&o[..m]);
        }
        self.pos = stop;
        if let Some(k) = change {
            self.end_live();
            if let Class::Run(next) = self.class(k, map)? {
                return self.begin(stop, next, map, read);
            }
        }
        Ok(true)
    }
}

fn stretch_error(e: pm_stretch::StretchError) -> AudioError {
    AudioError::Device(e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fair_slices_keep_native_read_calls_and_pcm_bit_exact() {
        for speed in [0.5, 0.75, 1.5, 1.6, 2.0] {
            for direction in [-1.0, 1.0] {
                let start = 96_000;
                let mut reference = StretchRun::new(2, 48_000, 0);
                let mut sliced = StretchRun::new(2, 48_000, 0);
                let mut whole_reads = Vec::new();
                let mut sliced_reads = Vec::new();
                let mut whole_map = |k: i64| {
                    Ok(MapPoint {
                        x: 200_000.0 + direction * k as f64 * CONTROL as f64 * speed,
                        on: true,
                    })
                };
                let mut sliced_map = |k: i64| {
                    Ok(MapPoint {
                        x: 200_000.0 + direction * k as f64 * CONTROL as f64 * speed,
                        on: true,
                    })
                };
                let mut whole_read = |lo: i64, n: usize, out: &mut [Vec<f32>]| {
                    whole_reads.push((lo, n));
                    sine(lo, n, out);
                    Ok(true)
                };
                let mut sliced_read = |lo: i64, n: usize, out: &mut [Vec<f32>]| {
                    sliced_reads.push((lo, n));
                    sine(lo, n, out);
                    Ok(true)
                };
                let mut expected = vec![Vec::new(); 2];
                let mut actual = vec![Vec::new(); 2];
                assert!(
                    reference
                        .render(start, 4096, &mut whole_map, &mut whole_read, &mut expected)
                        .unwrap()
                );
                let mut yields = 0;
                loop {
                    match sliced
                        .render_budgeted(
                            start,
                            4096,
                            &mut sliced_map,
                            &mut sliced_read,
                            &mut actual,
                            &mut || true,
                        )
                        .unwrap()
                    {
                        PrepareOutcome::Complete => break,
                        PrepareOutcome::Yielded => {
                            yields += 1;
                            assert!(yields < 32, "slice did not progress");
                        }
                        PrepareOutcome::MissingInput => panic!("decoded input reported missing"),
                    }
                }
                assert!(yields > 1, "pre-roll and process must both slice");
                assert_eq!(
                    whole_reads, sliced_reads,
                    "native input calls {speed} {direction}"
                );
                assert_eq!(reference.fresh_starts, sliced.fresh_starts);
                for (a, b) in expected.iter().zip(&actual) {
                    assert_eq!(
                        a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                        b.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                        "PCM {speed} {direction}"
                    );
                }
            }
        }
    }

    #[test]
    fn sliced_join_resumes_without_treating_cpu_as_missing_input() {
        let start = 96_000;
        let at = start + 1024;
        let mut reference = StretchRun::new(2, 48_000, 0);
        let mut sliced = StretchRun::new(2, 48_000, 0);
        let mut map = |k: i64| {
            Ok(MapPoint {
                x: 200_000.0 + k as f64 * CONTROL as f64 * 1.5,
                on: true,
            })
        };
        let mut whole_read = |lo, n, out: &mut [Vec<f32>]| {
            sine(lo, n, out);
            Ok(true)
        };
        let mut actual_read = |lo, n, out: &mut [Vec<f32>]| {
            sine(lo, n, out);
            Ok(true)
        };
        let mut expected = vec![Vec::new(); 2];
        let mut actual = vec![Vec::new(); 2];
        assert!(
            reference
                .prepare_join(start, at, &mut map, &mut whole_read, &mut expected)
                .unwrap()
        );
        let mut yields = 0;
        loop {
            match sliced
                .prepare_join_budgeted(
                    start,
                    at,
                    &mut map,
                    &mut actual_read,
                    &mut actual,
                    &mut || true,
                )
                .unwrap()
            {
                PrepareOutcome::Complete => break,
                PrepareOutcome::Yielded => {
                    yields += 1;
                    assert!(yields < 32, "join did not progress");
                }
                PrepareOutcome::MissingInput => panic!("decoded join input reported missing"),
            }
        }
        assert!(yields > 0);
        assert_eq!(actual, expected);
        assert_eq!(sliced.fresh_starts, reference.fresh_starts);

        let mut missed_once = false;
        let mut missing = |lo, n, out: &mut [Vec<f32>]| {
            if !missed_once {
                missed_once = true;
                return Ok(false);
            }
            sine(lo, n, out);
            Ok(true)
        };
        let mut retry = StretchRun::new(2, 48_000, 0);
        let mut never_missed = StretchRun::new(2, 48_000, 0);
        let mut retry_reference = vec![Vec::new(); 2];
        assert!(
            never_missed
                .render(start, 256, &mut map, &mut whole_read, &mut retry_reference)
                .unwrap()
        );
        assert_eq!(
            retry
                .render_budgeted(start, 256, &mut map, &mut missing, &mut actual, &mut || {
                    true
                })
                .unwrap(),
            PrepareOutcome::MissingInput
        );
        assert_eq!(retry.pending_retry(), Some((start, start + 256)));
        let mut retries = 0;
        loop {
            match retry
                .render_budgeted(start, 256, &mut map, &mut missing, &mut actual, &mut || {
                    true
                })
                .unwrap()
            {
                PrepareOutcome::Complete => break,
                PrepareOutcome::Yielded => {
                    retries += 1;
                    assert!(retries < 32);
                }
                PrepareOutcome::MissingInput => panic!("the one-time miss must recover"),
            }
        }
        assert!(retries > 0);
        for (a, b) in actual.iter().zip(&retry_reference) {
            assert_eq!(
                a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                b.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                "resumed PCM equals never-missing PCM"
            );
        }
    }

    fn render_all_slices(
        run: &mut StretchRun,
        at: i64,
        frames: usize,
        map: &mut MapFn<'_>,
        read: &mut ReadFn<'_>,
        out: &mut [Vec<f32>],
    ) -> usize {
        let mut yields = 0;
        loop {
            match run
                .render_budgeted(at, frames, map, read, out, &mut || true)
                .unwrap()
            {
                PrepareOutcome::Complete => return yields,
                PrepareOutcome::Yielded => {
                    yields += 1;
                    assert!(yields < 64, "native slices must advance");
                }
                PrepareOutcome::MissingInput => panic!("test input is fully decoded"),
            }
        }
    }

    fn assert_pcm_bits_equal(actual: &[Vec<f32>], expected: &[Vec<f32>]) {
        assert_eq!(actual.len(), expected.len());
        for (a, b) in actual.iter().zip(expected) {
            assert_eq!(
                a.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
                b.iter().map(|v| v.to_bits()).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn fair_slices_cross_forward_silence_and_reverse_classes_bit_exact() {
        let start: i64 = 96_000;
        let base = start.div_euclid(CONTROL);
        let map_point = |k: i64| {
            let rel = k - base;
            let forward = 200_000.0 + 6.0 * CONTROL as f64 * 1.5;
            MapPoint {
                x: if rel <= 6 {
                    200_000.0 + rel as f64 * CONTROL as f64 * 1.5
                } else if rel <= 9 {
                    forward
                } else {
                    forward - (rel - 9) as f64 * CONTROL as f64 * 1.5
                },
                on: true,
            }
        };
        let mut class_map = |k| Ok(map_point(k));
        let mut probe = StretchRun::new(2, 48_000, 0);
        assert_eq!(probe.class(base, &mut class_map).unwrap(), Class::Run(1));
        assert_eq!(
            probe.class(base + 6, &mut class_map).unwrap(),
            Class::Silent
        );
        assert_eq!(
            probe.class(base + 9, &mut class_map).unwrap(),
            Class::Run(-1)
        );

        let mut reference = StretchRun::new(2, 48_000, 0);
        let mut sliced = StretchRun::new(2, 48_000, 0);
        let mut whole_map = |k| Ok(map_point(k));
        let mut sliced_map = |k| Ok(map_point(k));
        let mut whole_reads = Vec::new();
        let mut sliced_reads = Vec::new();
        let mut whole_read = |lo, n, out: &mut [Vec<f32>]| {
            whole_reads.push((lo, n));
            sine(lo, n, out);
            Ok(true)
        };
        let mut sliced_read = |lo, n, out: &mut [Vec<f32>]| {
            sliced_reads.push((lo, n));
            sine(lo, n, out);
            Ok(true)
        };
        let mut expected = vec![Vec::new(); 2];
        let mut actual = vec![Vec::new(); 2];
        assert!(
            reference
                .render(start, 4096, &mut whole_map, &mut whole_read, &mut expected)
                .unwrap()
        );
        assert!(
            render_all_slices(
                &mut sliced,
                start,
                4096,
                &mut sliced_map,
                &mut sliced_read,
                &mut actual,
            ) > 1
        );
        assert_eq!(
            whole_reads, sliced_reads,
            "same native reads across classes"
        );
        assert_pcm_bits_equal(&actual, &expected);
        assert_eq!(sliced.fresh_starts, reference.fresh_starts);
    }

    #[test]
    fn a_sliced_loop_wrap_restarts_with_exact_fresh_phase() {
        let start = 96_000;
        let wrapped = start - 4096;
        let mut reference = StretchRun::new(2, 48_000, 0);
        let mut sliced = StretchRun::new(2, 48_000, 0);
        let mut map = |k: i64| {
            Ok(MapPoint {
                x: 200_000.0 + k as f64 * CONTROL as f64 * 1.5,
                on: true,
            })
        };
        let mut read = |lo, n, out: &mut [Vec<f32>]| {
            sine(lo, n, out);
            Ok(true)
        };
        let mut expected = vec![Vec::new(); 2];
        let mut actual = vec![Vec::new(); 2];
        assert!(
            reference
                .render(start, 4096, &mut map, &mut read, &mut expected)
                .unwrap()
        );
        assert!(render_all_slices(&mut sliced, start, 4096, &mut map, &mut read, &mut actual,) > 0);
        assert_pcm_bits_equal(&actual, &expected);
        assert!(
            reference
                .render(wrapped, 4096, &mut map, &mut read, &mut expected)
                .unwrap()
        );
        assert!(
            render_all_slices(&mut sliced, wrapped, 4096, &mut map, &mut read, &mut actual,) > 0
        );
        assert_pcm_bits_equal(&actual, &expected);
        assert_eq!(reference.fresh_starts, 2, "wrap starts a new phase");
        assert_eq!(sliced.fresh_starts, reference.fresh_starts);

        let mut fresh = StretchRun::new(2, 48_000, 0);
        let mut fresh_pcm = vec![Vec::new(); 2];
        assert!(
            fresh
                .render(wrapped, 4096, &mut map, &mut read, &mut fresh_pcm)
                .unwrap()
        );
        assert_pcm_bits_equal(&actual, &fresh_pcm);
    }

    #[test]
    fn shortening_the_prefix_changes_the_existing_offline_pcm() {
        let st = Stretcher::new(2, 48_000).unwrap();
        eprintln!(
            "A8 Signalsmith input/output latency: {}/{} samples; deterministic prefix {}",
            st.input_latency(),
            st.output_latency(),
            preroll(48_000)
        );
        for speed in [0.75, 1.5] {
            for start in [96_000, 144_000, 192_000] {
                let mut map = |k: i64| {
                    Ok(MapPoint {
                        x: k as f64 * CONTROL as f64 * speed,
                        on: true,
                    })
                };
                let mut read = |lo: i64, n: usize, out: &mut [Vec<f32>]| {
                    for (c, o) in out.iter_mut().enumerate() {
                        *o = (lo..lo + n as i64)
                            .map(|s| ((s as f64 * (0.057 + c as f64 * 0.013)).sin() * 0.2) as f32)
                            .collect();
                    }
                    Ok(true)
                };
                let mut reference = vec![vec![]; 2];
                StretchRun::new(2, 48_000, 0)
                    .render(start, 4096, &mut map, &mut read, &mut reference)
                    .unwrap();
                for pre in [
                    0,
                    5760,
                    5760 + st.output_latency() as i64,
                    preroll(48_000) - CONTROL,
                ] {
                    let mut shorter = StretchRun::new(2, 48_000, 0);
                    shorter.test_preroll = Some(pre);
                    let mut actual = vec![vec![]; 2];
                    shorter
                        .render(start, 4096, &mut map, &mut read, &mut actual)
                        .unwrap();
                    let different = reference
                        .iter()
                        .flatten()
                        .zip(actual.iter().flatten())
                        .filter(|(a, b)| a.to_bits() != b.to_bits())
                        .count();
                    eprintln!(
                        "A8 prefix {pre}, speed {speed}, start {start}: {different}/8192 changed values"
                    );
                    assert!(
                        different > 0,
                        "shortened prefix unexpectedly preserves existing PCM"
                    );
                }
            }
        }
    }

    #[test]
    fn missing_input_never_poison_stretch_history() {
        for dir in [1.0, -1.0] {
            // A miss during output_seek, and one after analysis has begun.
            for missing_read in [0, 4, 22] {
                let mut expected = StretchRun::new(2, 48_000, 0);
                let mut actual = StretchRun::new(2, 48_000, 0);
                let mut map = |k: i64| {
                    Ok(MapPoint {
                        x: 200_000.0 + dir * k as f64 * CONTROL as f64 * 1.5,
                        on: true,
                    })
                };
                let signal =
                    |lo: i64, n: usize, out: &mut [Vec<f32>]| {
                        for (c, channel) in out.iter_mut().enumerate() {
                            channel.clear();
                            channel.extend((lo..lo + n as i64).map(|s| {
                                ((s as f64 * (0.057 + c as f64 * 0.013)).sin() * 0.2) as f32
                            }));
                        }
                        Ok(true)
                    };
                let mut reads = 0;
                let mut delayed = |lo: i64, n: usize, out: &mut [Vec<f32>]| {
                    let missing = reads == missing_read;
                    reads += 1;
                    if missing {
                        for channel in out.iter_mut() {
                            channel.clear();
                            channel.resize(n, 0.0);
                        }
                        Ok(false)
                    } else {
                        signal(lo, n, out)
                    }
                };
                let mut reference = vec![Vec::new(); 2];
                let mut played = vec![Vec::new(); 2];
                let mut misses = 0;
                for start in (48_000..60_000).step_by(256) {
                    expected
                        .render(start, 256, &mut map, &mut { signal }, &mut reference)
                        .unwrap();
                    let complete = actual
                        .render(start, 256, &mut map, &mut delayed, &mut played)
                        .unwrap();
                    if complete {
                        assert_eq!(
                            played, reference,
                            "direction {dir}, missing read {missing_read}, at {start}"
                        );
                    } else {
                        misses += 1;
                        assert!(
                            played.iter().flatten().all(|&v| v == 0.0),
                            "incomplete stretch must be silent"
                        );
                    }
                }
                assert_eq!(misses, 1, "the missing input is reported, then recovered");
            }
        }
    }

    fn sine(lo: i64, n: usize, out: &mut [Vec<f32>]) {
        for (c, channel) in out.iter_mut().enumerate() {
            channel.clear();
            channel.extend(
                (lo..lo + n as i64)
                    .map(|s| ((s as f64 * (0.057 + c as f64 * 0.013)).sin() * 0.2) as f32),
            );
        }
    }

    /// A render that does not continue a starved one is a new request: it
    /// starts by the pre-roll rules (as an offline render from there does)
    /// and reads only that pre-roll, never the gap since the starve.
    #[test]
    fn a_seek_after_a_starve_starts_fresh() {
        let rate = 48_000;
        let mut map = |k: i64| {
            Ok(MapPoint {
                x: 200_000.0 + k as f64 * CONTROL as f64 * 1.5,
                on: true,
            })
        };
        let starve_at = 48_000;
        let n = 512;
        // Past wherever the run's read-ahead lets the starve land.
        let past = starve_at + 2 * GRID;
        for target in [
            // 60 s forward (the reviewer's probe), just past the starved
            // request, a grid step forward, and backward.
            starve_at + 60 * rate as i64,
            past + CONTROL,
            past + GRID,
            starve_at - GRID,
        ] {
            for abandoned in [false, true] {
                let reads = std::cell::Cell::new(0usize);
                let mut counted = |lo: i64, len: usize, out: &mut [Vec<f32>]| {
                    reads.set(reads.get() + len);
                    sine(lo, len, out);
                    Ok(true)
                };
                let mut fresh = StretchRun::new(2, rate, 0);
                let mut reference = vec![Vec::new(); 2];
                assert!(
                    fresh
                        .render(target, n, &mut map, &mut counted, &mut reference)
                        .unwrap()
                );
                let fresh_reads = reads.replace(0);

                let mut run = StretchRun::new(2, rate, 0);
                let mut out = vec![Vec::new(); 2];
                // Play a while, then starve.
                for start in (starve_at - 4 * n as i64..starve_at).step_by(n) {
                    assert!(
                        run.render(start, n, &mut map, &mut counted, &mut out)
                            .unwrap()
                    );
                }
                // Nothing new decodes during the starved render.
                let mut missing = |_: i64, len: usize, out: &mut [Vec<f32>]| {
                    for channel in out.iter_mut() {
                        channel.clear();
                        channel.resize(len, 0.0);
                    }
                    Ok(false)
                };
                // Play on (from what was produced ahead) until a render
                // needs new input.
                let mut at = starve_at;
                while run.render(at, n, &mut map, &mut missing, &mut out).unwrap() {
                    at += n as i64;
                    assert!(at < starve_at + 4 * GRID);
                }
                assert_eq!(run.pending_retry(), Some((at, at + n as i64)));
                assert!(target < at || target > at + n as i64);
                if abandoned {
                    run.abandon_retry();
                }
                reads.set(0);
                let starts = run.fresh_starts;
                assert!(
                    run.render(target, n, &mut map, &mut counted, &mut out)
                        .unwrap()
                );
                assert_eq!(
                    run.fresh_starts,
                    starts + 1,
                    "{target}: a new request starts fresh"
                );
                assert_eq!(
                    reads.get(),
                    fresh_reads,
                    "{target}: reads only the pre-roll, not the gap"
                );
                assert_eq!(out, reference, "{target}: equals a render from the target");
            }
        }
    }

    /// Playback retries a starved request (or a caller moves on to where it
    /// ended): either continues the run and equals an unstarved render.
    #[test]
    fn a_retry_after_a_starve_continues_the_run() {
        let rate = 48_000;
        let mut map = |k: i64| {
            Ok(MapPoint {
                x: 200_000.0 + k as f64 * CONTROL as f64 * 1.5,
                on: true,
            })
        };
        let n = 512;
        let starve_at = 48_000 + 3 * n as i64;
        let mut expected = StretchRun::new(2, rate, 0);
        let mut run = StretchRun::new(2, rate, 0);
        let mut reference = vec![Vec::new(); 2];
        let mut out = vec![Vec::new(); 2];
        let mut whole = |lo: i64, len: usize, out: &mut [Vec<f32>]| {
            sine(lo, len, out);
            Ok(true)
        };
        let mut missed = false;
        let mut once = |lo: i64, len: usize, out: &mut [Vec<f32>]| {
            if !missed && lo + len as i64 > starve_at + CONTROL {
                missed = true;
                for channel in out.iter_mut() {
                    channel.clear();
                    channel.resize(len, 0.0);
                }
                return Ok(false);
            }
            sine(lo, len, out);
            Ok(true)
        };
        let mut start = 48_000;
        let mut retries = 0;
        while start < 48_000 + 12 * n as i64 {
            if run.render(start, n, &mut map, &mut once, &mut out).unwrap() {
                expected
                    .render(start, n, &mut map, &mut whole, &mut reference)
                    .unwrap();
                assert_eq!(out, reference, "at {start}");
                start += n as i64;
            } else {
                retries += 1;
                assert!(retries < 3, "a retry with the input decoded completes");
            }
        }
        assert_eq!(retries, 1);
        assert_eq!(run.fresh_starts, 1, "the retry continued the run");
    }
    #[test]
    fn short_committed_drops_reenter_with_uninterrupted_phase_history() {
        for direction in [-1.0, 1.0] {
            let mut map = |k: i64| {
                Ok(MapPoint {
                    x: 200_000.0 + direction * k as f64 * CONTROL as f64 * 1.5,
                    on: true,
                })
            };
            let mut whole = |lo, len, out: &mut [Vec<f32>]| {
                sine(lo, len, out);
                Ok(true)
            };
            let mut missing = |_lo, len, out: &mut [Vec<f32>]| {
                for c in out {
                    c.clear();
                    c.resize(len, 0.0);
                }
                Ok(false)
            };
            let mut run = StretchRun::new(2, 48_000, 0);
            let mut reference = StretchRun::new(2, 48_000, 0);
            let mut out = vec![vec![]; 2];
            let mut expected = vec![vec![]; 2];
            let mut drops = 0;
            for a in (48_000..68_000).step_by(256) {
                reference
                    .render(a, 256, &mut map, &mut whole, &mut expected)
                    .unwrap();
                let complete = if (52_000..56_000).contains(&a) {
                    run.render(a, 256, &mut map, &mut missing, &mut out)
                        .unwrap()
                } else {
                    run.render(a, 256, &mut map, &mut whole, &mut out).unwrap()
                };
                if complete {
                    assert_eq!(out, expected, "{direction} at {a}");
                } else {
                    drops += 1;
                    run.commit_drop(a);
                    assert!(out.iter().flatten().all(|&v| v == 0.0));
                }
            }
            assert!(drops > 0);
            assert_eq!(
                run.fresh_starts, 1,
                "a dropped span keeps valid phase state"
            );
        }
    }

    /// A4 point 3: past the old fixed 120 ms window, a committed drop still
    /// re-enters with the uninterrupted phase history (equal to a render
    /// that never dropped) while the catch-up costs no more than a fresh
    /// start's worst case; a longer drop starts fresh by the pre-roll rules
    /// (equal to a render started at the re-entry, as a seek there).
    #[test]
    fn committed_drops_continue_within_the_catch_up_limit() {
        let rate = 48_000u32;
        let limit = catch_up_limit(rate);
        assert_eq!(limit, GRID + preroll(rate));
        for direction in [-1.0, 1.0] {
            // The last gap ends on a grid boundary, where a run that kept
            // its failed fresh start would begin a grid step early.
            let (from, drop_at) = (3 * GRID, 3 * GRID + 4_096);
            for (gap_ms, continues) in [(300i64, true), (500, true), (1_000, false), (1_280, false)]
            {
                let gap = gap_ms * rate as i64 / 1000;
                assert_eq!(gap + 256 <= limit, continues, "{gap_ms} ms vs the limit");
                let mut map = |k: i64| {
                    Ok(MapPoint {
                        x: 400_000.0 + direction * k as f64 * CONTROL as f64 * 1.5,
                        on: true,
                    })
                };
                let mut whole = |lo, len, out: &mut [Vec<f32>]| {
                    sine(lo, len, out);
                    Ok(true)
                };
                let mut missing = |_lo, len, out: &mut [Vec<f32>]| {
                    for c in out {
                        c.clear();
                        c.resize(len, 0.0);
                    }
                    Ok(false)
                };
                let mut run = StretchRun::new(2, rate, 0);
                let mut out = vec![vec![]; 2];
                let back = drop_at + gap;
                let mut played = Vec::new();
                for a in (from..back + 8_192).step_by(256) {
                    let complete = if (drop_at..back).contains(&a) {
                        run.render(a, 256, &mut map, &mut missing, &mut out)
                            .unwrap()
                    } else {
                        run.render(a, 256, &mut map, &mut whole, &mut out).unwrap()
                    };
                    if complete {
                        played.push((a, out.clone()));
                    } else {
                        assert!(a >= drop_at, "{direction} {gap_ms} ms at {a}");
                        run.commit_drop(a);
                    }
                }
                if continues {
                    assert_eq!(run.fresh_starts, 1, "{direction} {gap_ms} ms");
                } else {
                    assert!(run.fresh_starts >= 2, "{direction} {gap_ms} ms");
                }
                // One uninterrupted run from `from`, rendered through the
                // gap (a run plays on from what it produced ahead before a
                // render needs new input). Fresh: a new run from the
                // re-entry, as a seek there renders.
                let mut reference = StretchRun::new(2, rate, 0);
                let mut expected = vec![vec![]; 2];
                let mut heard = played.iter().peekable();
                for a in (from..back + 8_192).step_by(256) {
                    if (back..back + 256).contains(&a) && !continues {
                        reference = StretchRun::new(2, rate, 0);
                    }
                    reference
                        .render(a, 256, &mut map, &mut whole, &mut expected)
                        .unwrap();
                    if let Some((_, out)) = heard.next_if(|(at, _)| *at == a) {
                        assert_eq!(out, &expected, "{direction} {gap_ms} ms at {a}");
                    }
                }
                assert!(heard.next().is_none(), "every render compared");
                assert!(played.iter().any(|(a, _)| *a >= back));
            }
        }
    }

    /// Measurement (A4 point 3), not a pass/fail check: what exact vocoder
    /// continuity across a dropped gap costs. Continuing the phase history
    /// exactly means analysing every skipped interval with its real input,
    /// i.e. rendering the whole gap; a fresh start reads only its pre-roll.
    /// Run with `cargo test --release -p pm-audio --lib
    /// stretch::tests::measure_gap_catch_up -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn measure_gap_catch_up() {
        let rate = 48_000;
        let n = 512usize;
        let mut map = |k: i64| {
            Ok(MapPoint {
                x: 200_000.0 + k as f64 * CONTROL as f64 * 1.5,
                on: true,
            })
        };
        let from = 48_000i64;
        eprintln!("gap_s, catch_up_reads, catch_up_ms, fresh_reads, fresh_ms");
        for gap_ms in [120i64, 500, 1_000, 10_000, 60_000] {
            let target = from + gap_ms * rate as i64 / 1000;
            let reads = std::cell::Cell::new(0usize);
            let mut counted = |lo: i64, len: usize, out: &mut [Vec<f32>]| {
                reads.set(reads.get() + len);
                sine(lo, len, out);
                Ok(true)
            };
            let mut out = vec![Vec::new(); 2];
            // A run playing up to the drop.
            let mut run = StretchRun::new(2, rate, 0);
            for start in (from - 4 * n as i64..from).step_by(n) {
                assert!(
                    run.render(start, n, &mut map, &mut counted, &mut out)
                        .unwrap()
                );
            }
            reads.set(0);
            let t = std::time::Instant::now();
            let mut at = from;
            while at < target {
                let len = n.min((target - at) as usize);
                assert!(
                    run.render(at, len, &mut map, &mut counted, &mut out)
                        .unwrap()
                );
                at += len as i64;
            }
            assert!(
                run.render(target, n, &mut map, &mut counted, &mut out)
                    .unwrap()
            );
            let catch_up = (reads.replace(0), t.elapsed());
            let mut fresh = StretchRun::new(2, rate, 0);
            let t = std::time::Instant::now();
            assert!(
                fresh
                    .render(target, n, &mut map, &mut counted, &mut out)
                    .unwrap()
            );
            let fresh = (reads.get(), t.elapsed());
            eprintln!(
                "{:.3}, {}, {:.1}, {}, {:.1}",
                gap_ms as f64 / 1000.0,
                catch_up.0,
                catch_up.1.as_secs_f64() * 1000.0,
                fresh.0,
                fresh.1.as_secs_f64() * 1000.0
            );
        }
    }
}
