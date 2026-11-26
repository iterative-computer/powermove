//! Frame pacing, aggregated on the Mac: each frame is one relaxed atomic add
//! into a fixed histogram per context; the telemetry thread reads and clears
//! them for `perf_summary`. Memory, Metal, thermal and power are sampled by
//! the telemetry thread itself.
use crate::events::{ContextFrames, Frames, PerfContext};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// Upper edges (ms) of the histogram buckets; the last bucket is open. 8.33
/// and 16.67 are edges, so hitch counts are exact.
const EDGES: [f32; 20] = [
    1., 2., 3., 4., 5., 6., 7., 8.33, 10., 12.5, 16.67, 20., 25., 33.3, 50., 66.7, 100., 200.,
    500., 1000.,
];
const BUCKETS: usize = EDGES.len() + 1;

#[allow(clippy::declare_interior_mutable_const)]
const ZERO: AtomicU64 = AtomicU64::new(0);
#[allow(clippy::declare_interior_mutable_const)]
const ROW: [AtomicU64; BUCKETS] = [ZERO; BUCKETS];
static HISTOGRAM: [[AtomicU64; BUCKETS]; 5] = [ROW; 5];
static PLAY_FRAMES: AtomicU64 = AtomicU64::new(0);
static PLAY_MICROS: AtomicU64 = AtomicU64::new(0);
/// The rate playback aims for (f32 bits; 0 unknown).
static PLAY_TARGET: AtomicU64 = AtomicU64::new(0);
/// The last displayed played frame (host seconds, f64 bits).
static PLAY_LAST: AtomicU64 = AtomicU64::new(0);
/// 1 playing, 2 scrubbing; and when (main_thread clock, ms).
static MOMENT: AtomicU64 = AtomicU64::new(0);
static MOMENT_AT: AtomicU64 = AtomicU64::new(0);
/// 1 drag, 2 scroll; and when.
static INPUT: AtomicU64 = AtomicU64::new(0);
static INPUT_AT: AtomicU64 = AtomicU64::new(0);

/// What the person is doing now: playback and scrubbing from the viewer's
/// last frames, then a drag or scroll in the last moment.
pub fn context() -> PerfContext {
    let now = crate::main_thread::now_ms();
    if now.saturating_sub(MOMENT_AT.load(Relaxed)) < 500 {
        match MOMENT.load(Relaxed) {
            1 => return PerfContext::Playing,
            2 => return PerfContext::Scrubbing,
            _ => {}
        }
    }
    if now.saturating_sub(INPUT_AT.load(Relaxed)) < 150 {
        match INPUT.load(Relaxed) {
            1 => return PerfContext::Dragging,
            2 => return PerfContext::Scrolling,
            _ => {}
        }
    }
    PerfContext::Paused
}

#[inline]
pub fn note_playback_target(fps: f32) {
    if super::taking() && fps.is_finite() && fps > 0. {
        PLAY_TARGET.store(fps.to_bits() as u64, Relaxed);
    }
}

#[inline]
pub fn note_input(kind: u8) {
    if super::taking() {
        INPUT.store(kind as u64, Relaxed);
        INPUT_AT.store(crate::main_thread::now_ms(), Relaxed);
    }
}

#[inline]
pub fn note_viewer_frame(playing: bool, scrubbing: bool, displayed_at: Option<f64>) {
    if super::taking() {
        viewer_frame(playing, scrubbing, displayed_at);
    }
}

fn viewer_frame(playing: bool, scrubbing: bool, displayed_at: Option<f64>) {
    if !(playing || scrubbing) {
        return;
    }
    MOMENT.store(if playing { 1 } else { 2 }, Relaxed);
    MOMENT_AT.store(crate::main_thread::now_ms(), Relaxed);
    if let (true, Some(at)) = (playing, displayed_at) {
        let last = f64::from_bits(PLAY_LAST.swap(at.to_bits(), Relaxed));
        // A gap longer than a quarter second is a new stretch of playback.
        if last > 0. && at > last && at - last < 0.25 {
            PLAY_MICROS.fetch_add(((at - last) * 1e6) as u64, Relaxed);
            PLAY_FRAMES.fetch_add(1, Relaxed);
        }
    }
}

fn index(context: PerfContext) -> usize {
    PerfContext::ALL
        .iter()
        .position(|c| *c == context)
        .unwrap_or(0)
}

#[inline]
pub fn frame(context: PerfContext, ms: f32) {
    if super::taking() {
        record(context, ms);
    }
}

fn record(context: PerfContext, ms: f32) {
    let bucket = EDGES
        .iter()
        .position(|edge| ms <= *edge)
        .unwrap_or(EDGES.len());
    HISTOGRAM[index(context)][bucket].fetch_add(1, Relaxed);
}

/// Sharing turned off: everything counted goes.
pub fn clear() {
    let _ = take();
    PLAY_LAST.store(0, Relaxed);
}

/// Reads and clears the histograms.
/// The histograms, and playback's achieved and target rates (when it played).
pub fn take() -> (Frames, Option<(f32, Option<f32>)>) {
    let mut frames = Frames::default();
    for (slot, histogram) in frames.0.iter_mut().zip(&HISTOGRAM) {
        let counts: Vec<u64> = histogram.iter().map(|b| b.swap(0, Relaxed)).collect();
        *slot = summarize(&counts);
    }
    let played = PLAY_FRAMES.swap(0, Relaxed);
    let micros = PLAY_MICROS.swap(0, Relaxed);
    let target = f32::from_bits(PLAY_TARGET.load(Relaxed) as u32);
    let playback = (micros > 0).then(|| {
        (
            (played as f64 / (micros as f64 / 1e6)) as f32,
            (target > 0.).then_some(target),
        )
    });
    (frames, playback)
}

/// Percentiles from bucket counts: each lands on its bucket's upper edge
/// (the open bucket reads as twice the last edge).
pub fn summarize(counts: &[u64]) -> Option<ContextFrames> {
    let total: u64 = counts.iter().sum();
    if total == 0 {
        return None;
    }
    let quantile = |q: f64| {
        let rank = (q * total as f64).ceil().max(1.) as u64;
        let mut seen = 0;
        for (i, c) in counts.iter().enumerate() {
            seen += c;
            if seen >= rank {
                return EDGES.get(i).copied().unwrap_or(EDGES[EDGES.len() - 1] * 2.);
            }
        }
        EDGES[EDGES.len() - 1] * 2.
    };
    let over = |ms: f32| -> u64 {
        let first = EDGES.iter().position(|e| *e > ms).unwrap_or(EDGES.len());
        counts[first..].iter().sum()
    };
    Some(ContextFrames {
        frames: total,
        p50_ms: quantile(0.5),
        p95_ms: quantile(0.95),
        p99_ms: quantile(0.99),
        hitches_8ms: over(8.33),
        hitches_16ms: over(16.67),
    })
}

/// Peaks over a summary window, sampled by the telemetry thread.
#[derive(Default)]
pub struct Peaks {
    pub footprint: u64,
    pub metal: Option<u64>,
    pub thermal: u8,
    pub on_battery: bool,
}

impl Peaks {
    pub fn sample(&mut self, metal: Option<&(dyn Fn() -> u64 + Send + Sync)>) {
        self.footprint = self.footprint.max(super::system::footprint());
        if let Some(metal) = metal {
            self.metal = Some(self.metal.unwrap_or(0).max(metal()));
        }
        self.thermal = self.thermal.max(super::system::thermal_state());
        self.on_battery |= super::system::on_battery();
    }
}

pub fn thermal_name(state: u8) -> &'static str {
    match state {
        0 => "nominal",
        1 => "fair",
        2 => "serious",
        _ => "critical",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_and_hitches_come_from_the_buckets() {
        let mut counts = vec![0u64; BUCKETS];
        counts[7] = 90; // ≤ 8.33 ms
        counts[9] = 8; // ≤ 12.5 ms
        counts[11] = 2; // ≤ 20 ms
        let s = summarize(&counts).unwrap();
        assert_eq!(s.frames, 100);
        assert_eq!(s.p50_ms, 8.33);
        assert_eq!(s.p95_ms, 12.5);
        assert_eq!(s.p99_ms, 20.);
        assert_eq!(s.hitches_8ms, 10);
        assert_eq!(s.hitches_16ms, 2);
        assert!(summarize(&[0; BUCKETS]).is_none());
    }

    #[test]
    fn a_frame_lands_in_its_context_and_take_clears_it() {
        let _global = super::super::tests::GLOBAL
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        record(PerfContext::Scrolling, 9.);
        record(PerfContext::Scrolling, 4.);
        PLAY_TARGET.store(60f32.to_bits() as u64, Relaxed);
        for n in 0..=60 {
            viewer_frame(true, false, Some(100. + n as f64 / 60.));
        }
        assert_eq!(context(), PerfContext::Playing);
        let (frames, playback) = take();
        let scrolling = frames.0[4].as_ref().unwrap();
        assert_eq!((scrolling.frames, scrolling.hitches_8ms), (2, 1));
        assert!(
            playback.is_some_and(|(fps, target)| (fps - 60.).abs() < 0.5 && target == Some(60.)),
            "{playback:?}"
        );
        assert!(take().0.0[4].is_none());
    }
}
