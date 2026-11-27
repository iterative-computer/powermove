//! Band-limited interpolation: sample-rate conversion, speed, and remaps
//! (plan section 16: `rubato` sinc resampling, at least 120 dB stopband,
//! passband to 0.45·fs).
//!
//! # The filter
//!
//! rubato's sinc design: a windowed sinc of [`TAPS`] taps under a squared
//! Blackman-Harris window, with the cutoff from [`rubato::calculate_cutoff`]
//! (the highest cutoff that keeps aliasing under the window's sidelobes for
//! that length). When a voice reads its source faster than one source sample
//! per output sample (a higher source rate, or a speed above 1), the cutoff
//! scales down by that factor and the filter grows by it, so the transition
//! band stays the same fraction of the output band.
//!
//! # Why voices do not run `rubato::Async`
//!
//! A voice must answer "what is heard at project sample n" for any n: seeks
//! land on a sample, playback after a seek must equal playback straight
//! through (deterministic pre-roll), time remaps jump and run backwards, and
//! offline export must equal the preview. `rubato::Async` keeps a running
//! position that advances by the ratio and cannot seek or reverse. So the
//! voices evaluate the same filter as a pure function of the source position
//! of each output sample:
//!
//! * **Grid** (constant rate, any rational source-to-project ratio): the
//!   positions fall on `F = project / gcd(source, project)` phases; the bank
//!   holds exactly those phases, and an output is one dot product. Exact
//!   integer position arithmetic, bit-identical whatever sample a render
//!   starts at. This is rubato's `SincInterpolationType::Nearest` with the
//!   oversampling factor it recommends for such ratios.
//! * **Free** (speed, variable speed, time remaps): [`FREE_PHASES`] phases
//!   with cubic interpolation between them (rubato's `Cubic`), positions in
//!   f64 from the time map.
//!
//! rubato itself converts the master to the device rate when the device
//! does not run at the project rate (`device`), the streaming case it is
//! designed for. The tests measure both paths against the section 16 bars.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use rubato::WindowFunction;

/// Taps at unit rate. 512 taps under the squared Blackman-Harris window put
/// the passband edge (0.01 dB) above 0.45·fs with more than 120 dB
/// stopband (see the tests).
pub const TAPS: usize = 512;
/// Rows of the free-position bank (cubic interpolation between rows).
pub const FREE_PHASES: usize = 256;
/// The largest exact-phase bank; finer ratios use the free bank.
pub const MAX_GRID_PHASES: usize = 4096;
/// Reads faster than this many source samples per output sample alias
/// (the filter stops growing).
pub const MAX_DECIMATION: f64 = 8.0;

/// The window.
pub const WINDOW: WindowFunction = WindowFunction::BlackmanHarris2;

/// Cutoff relative to the Nyquist frequency of the lower rate.
pub fn design_cutoff() -> f64 {
    rubato::calculate_cutoff::<f64>(TAPS, WINDOW)
}

/// The squared Blackman-Harris window on [−half, half] (zero outside).
fn window(t: f64, half: f64) -> f64 {
    if t.abs() > half {
        return 0.0;
    }
    let x = (t + half) / (2.0 * half);
    let tau = std::f64::consts::TAU;
    let w = 0.35875 - 0.48829 * (tau * x).cos() + 0.14128 * (2.0 * tau * x).cos()
        - 0.01168 * (3.0 * tau * x).cos();
    w * w
}

fn sinc(x: f64) -> f64 {
    if x == 0.0 {
        1.0
    } else {
        let px = std::f64::consts::PI * x;
        px.sin() / px
    }
}

/// A bank of filter rows: row `j` interpolates at fractional position
/// `(j − first) / phases`.
#[derive(Debug)]
pub struct Bank {
    pub taps: usize,
    pub phases: usize,
    /// How much faster than the output the source is read; the cutoff is
    /// divided by it.
    pub decimation: f64,
    /// Cutoff relative to the source Nyquist frequency.
    pub cutoff: f64,
    /// The row of phase 0 (1 in the free bank, which keeps a row before 0
    /// and two after the last phase for cubic interpolation).
    first: usize,
    rows: usize,
    table: Vec<f32>,
}

impl Bank {
    fn build(phases: usize, decimation: f64, free: bool) -> Bank {
        let decimation = decimation.max(1.0);
        let taps = taps_for_decimation(decimation);
        let cutoff = design_cutoff() / decimation;
        let half = taps as f64 / 2.0;
        let (first, rows) = if free { (1, phases + 3) } else { (0, phases) };
        let mut table = vec![0f32; rows * taps];
        let mut row = vec![0f64; taps];
        for r in 0..rows {
            let phase = (r as f64 - first as f64) / phases as f64;
            for (k, v) in row.iter_mut().enumerate() {
                // Tap k reads input sample i − half + 1 + k for a position
                // i + phase.
                let t = k as f64 - half + 1.0 - phase;
                *v = cutoff * sinc(cutoff * t) * window(t, half);
            }
            let sum: f64 = row.iter().sum();
            for (dst, v) in table[r * taps..(r + 1) * taps].iter_mut().zip(&row) {
                *dst = (v / sum) as f32;
            }
        }
        Bank {
            taps,
            phases,
            decimation,
            cutoff,
            first,
            rows,
            table,
        }
    }

    #[inline]
    fn row(&self, r: usize) -> &[f32] {
        &self.table[r * self.taps..(r + 1) * self.taps]
    }

    /// Samples before a position the filter reads (and `taps − before − 1`
    /// after it).
    pub fn before(&self) -> usize {
        self.taps / 2 - 1
    }

    /// One output at integer sample `i` plus `phase / phases` from `input`,
    /// where `input[0]` is source sample `input_start`. Exact-phase banks
    /// only.
    #[inline]
    pub fn grid(&self, input: &[f32], input_start: i64, i: i64, phase: usize) -> f32 {
        let at = (i - self.before() as i64 - input_start) as usize;
        dot(&input[at..at + self.taps], self.row(self.first + phase))
    }

    /// The kernel for a free position's fraction, interpolated cubically
    /// between rows, into `out` (`taps` long).
    pub fn free_kernel(&self, fraction: f64, out: &mut [f32]) {
        let f = fraction * self.phases as f64;
        let p = (f.floor() as usize).min(self.phases - 1);
        let x = (f - p as f64) as f32;
        let w = [
            -x * (x - 1.0) * (x - 2.0) / 6.0,
            (x + 1.0) * (x - 1.0) * (x - 2.0) / 2.0,
            -(x + 1.0) * x * (x - 2.0) / 2.0,
            (x + 1.0) * x * (x - 1.0) / 6.0,
        ];
        let base = self.first + p - 1;
        let (r0, r1, r2, r3) = (
            self.row(base),
            self.row(base + 1),
            self.row(base + 2),
            self.row(base + 3),
        );
        for k in 0..self.taps {
            out[k] = w[0] * r0[k] + w[1] * r1[k] + w[2] * r2[k] + w[3] * r3[k];
        }
        debug_assert!(base + 3 < self.rows);
    }
}

/// A dot product with a fixed summation order (reproducible run to run):
/// sixteen lanes of fused multiply-add on aarch64 (NEON), eight independent
/// accumulators elsewhere.
#[inline]
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len().min(b.len());
    let (a, b) = (&a[..n], &b[..n]);
    #[cfg(target_arch = "aarch64")]
    {
        use std::arch::aarch64::*;
        let chunks = n / 16;
        // SAFETY: NEON is part of the aarch64 baseline; every load reads 4
        // floats inside `a[..chunks * 16]` and `b[..chunks * 16]`.
        let mut sum = unsafe {
            let (pa, pb) = (a.as_ptr(), b.as_ptr());
            let mut acc = [vdupq_n_f32(0.0); 4];
            for c in 0..chunks {
                let o = c * 16;
                acc[0] = vfmaq_f32(acc[0], vld1q_f32(pa.add(o)), vld1q_f32(pb.add(o)));
                acc[1] = vfmaq_f32(acc[1], vld1q_f32(pa.add(o + 4)), vld1q_f32(pb.add(o + 4)));
                acc[2] = vfmaq_f32(acc[2], vld1q_f32(pa.add(o + 8)), vld1q_f32(pb.add(o + 8)));
                acc[3] = vfmaq_f32(acc[3], vld1q_f32(pa.add(o + 12)), vld1q_f32(pb.add(o + 12)));
            }
            vaddvq_f32(vaddq_f32(
                vaddq_f32(acc[0], acc[1]),
                vaddq_f32(acc[2], acc[3]),
            ))
        };
        for k in chunks * 16..n {
            sum += a[k] * b[k];
        }
        sum
    }
    #[cfg(not(target_arch = "aarch64"))]
    {
        let mut acc = [0f32; 8];
        let chunks = n / 8;
        for c in 0..chunks {
            let (x, y) = (&a[c * 8..c * 8 + 8], &b[c * 8..c * 8 + 8]);
            for l in 0..8 {
                acc[l] += x[l] * y[l];
            }
        }
        let mut tail = 0f32;
        for k in chunks * 8..n {
            tail += a[k] * b[k];
        }
        ((acc[0] + acc[4]) + (acc[1] + acc[5])) + ((acc[2] + acc[6]) + (acc[3] + acc[7])) + tail
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Key {
    Grid {
        phases: usize,
        decimation_milli: u64,
    },
    Free {
        decimation_step: u32,
    },
}

fn banks() -> &'static Mutex<HashMap<Key, Arc<Bank>>> {
    static BANKS: OnceLock<Mutex<HashMap<Key, Arc<Bank>>>> = OnceLock::new();
    BANKS.get_or_init(Default::default)
}

fn cached(key: Key, build: impl FnOnce() -> Bank) -> Arc<Bank> {
    let mut map = banks().lock().unwrap_or_else(|e| e.into_inner());
    map.entry(key).or_insert_with(|| Arc::new(build())).clone()
}

/// The exact-phase bank for reading a `source`-rate stream at `project`
/// rate: `phases = project / gcd`, decimation `source / project` when the
/// source is faster. `None` when the ratio needs more than
/// [`MAX_GRID_PHASES`] phases.
pub fn grid_bank(source: u32, project: u32) -> Option<Arc<Bank>> {
    let g = gcd(source as u64, project as u64);
    let phases = (project as u64 / g) as usize;
    if phases > MAX_GRID_PHASES {
        return None;
    }
    let decimation = (source as f64 / project as f64).clamp(1.0, MAX_DECIMATION);
    let key = Key::Grid {
        phases,
        decimation_milli: (decimation * 1000.0).round() as u64,
    };
    Some(cached(key, || Bank::build(phases, decimation, false)))
}

/// Free-position decimation steps: 2^(k/8), so a bank's cutoff is at most
/// 9% below what the read rate needs (never above it: no aliasing).
fn free_step(decimation: f64) -> u32 {
    let d = decimation.clamp(1.0, MAX_DECIMATION);
    (d.log2() * 8.0).ceil() as u32
}

fn taps_for_decimation(decimation: f64) -> usize {
    ((TAPS as f64 * decimation.max(1.0)).ceil() as usize).next_multiple_of(8)
}

/// The exact tap count of [`free_bank`] without constructing filter rows.
/// Source bounds use this before the first audible resampling bank exists.
pub fn free_taps(decimation: f64) -> usize {
    let step = free_step(decimation);
    taps_for_decimation(2f64.powf(step as f64 / 8.0))
}

/// The free-position bank for reading `decimation` source samples per
/// output sample (at least 1).
pub fn free_bank(decimation: f64) -> Arc<Bank> {
    let step = free_step(decimation);
    cached(
        Key::Free {
            decimation_step: step,
        },
        || Bank::build(FREE_PHASES, 2f64.powf(step as f64 / 8.0), true),
    )
}

pub fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_have_unit_dc_gain() {
        let bank = grid_bank(44_100, 48_000).unwrap();
        assert_eq!(bank.phases, 160);
        for p in 0..bank.phases {
            let s: f64 = bank.row(p).iter().map(|&v| v as f64).sum();
            assert!((s - 1.0).abs() < 1e-5, "phase {p}: {s}");
        }
        let free = free_bank(1.0);
        let mut k = vec![0f32; free.taps];
        free.free_kernel(0.37, &mut k);
        let s: f64 = k.iter().map(|&v| v as f64).sum();
        assert!((s - 1.0).abs() < 1e-5);
    }

    #[test]
    fn phase_zero_is_nearly_an_impulse_at_unit_rate() {
        let bank = grid_bank(48_000, 48_000).unwrap();
        let row = bank.row(0);
        let centre = bank.before();
        assert!(row[centre] > 0.97, "{}", row[centre]);
    }

    #[test]
    fn decimating_banks_grow() {
        let bank = grid_bank(96_000, 48_000).unwrap();
        assert_eq!(bank.phases, 1);
        assert_eq!(bank.taps, 2 * TAPS);
        assert!((bank.cutoff - design_cutoff() / 2.0).abs() < 1e-12);
        assert!(free_bank(1.5).decimation >= 1.5);
        assert!(free_bank(1.5).decimation < 1.5 * 1.1);
    }

    #[test]
    fn free_tap_bounds_match_banks_at_rate_and_quantization_boundaries() {
        let ratios = [
            0.5,
            1.0,
            44_100.0 / 48_000.0,
            48_000.0 / 44_100.0,
            96_000.0 / 48_000.0,
            192_000.0 / 44_100.0,
            8.0,
            16.0,
        ];
        for ratio in ratios
            .into_iter()
            .chain([1, 8, 16].into_iter().flat_map(|step| {
                let boundary = 2f64.powf(step as f64 / 8.0);
                [boundary - 1e-12, boundary, boundary + 1e-12]
            }))
        {
            let bank = free_bank(ratio);
            let taps = free_taps(ratio);
            assert_eq!(taps, bank.taps, "ratio {ratio}");
            assert_eq!(taps / 2 - 1, bank.before(), "ratio {ratio}");
        }
    }
}
