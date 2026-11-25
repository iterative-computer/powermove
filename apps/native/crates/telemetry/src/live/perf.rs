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
/// The last target rate, as f32 bits.
static PLAY_TARGET: AtomicU64 = AtomicU64::new(0);

fn index(context: PerfContext) -> usize {
    PerfContext::ALL
        .iter()
        .position(|c| *c == context)
        .unwrap_or(0)
}

#[inline]
pub fn frame(context: PerfContext, ms: f32) {
    if !super::on() {
        return;
    }
    let bucket = EDGES
        .iter()
        .position(|edge| ms <= *edge)
        .unwrap_or(EDGES.len());
    HISTOGRAM[index(context)][bucket].fetch_add(1, Relaxed);
}

#[inline]
pub fn playback(frames: u64, seconds: f64, target_fps: f32) {
    if !super::on() || seconds <= 0. {
        return;
    }
    PLAY_FRAMES.fetch_add(frames, Relaxed);
    PLAY_MICROS.fetch_add((seconds * 1e6) as u64, Relaxed);
    PLAY_TARGET.store(target_fps.to_bits() as u64, Relaxed);
}

/// Reads and clears the histograms.
pub fn take() -> (Frames, Option<(f32, f32)>) {
    let mut frames = Frames::default();
    for (slot, histogram) in frames.0.iter_mut().zip(&HISTOGRAM) {
        let counts: Vec<u64> = histogram.iter().map(|b| b.swap(0, Relaxed)).collect();
        *slot = summarize(&counts);
    }
    let played = PLAY_FRAMES.swap(0, Relaxed);
    let micros = PLAY_MICROS.swap(0, Relaxed);
    let playback = (micros > 0).then(|| {
        (
            (played as f64 / (micros as f64 / 1e6)) as f32,
            f32::from_bits(PLAY_TARGET.load(Relaxed) as u32),
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
        super::super::force_on_for_tests();
        frame(PerfContext::Scrolling, 9.);
        frame(PerfContext::Scrolling, 4.);
        playback(60, 1., 60.);
        let (frames, playback) = take();
        let scrolling = frames.0[4].as_ref().unwrap();
        assert_eq!((scrolling.frames, scrolling.hitches_8ms), (2, 1));
        assert_eq!(playback, Some((60., 60.)));
        assert!(take().0.0[4].is_none());
    }
}
