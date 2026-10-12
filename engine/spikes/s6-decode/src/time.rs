//! Exact time for the spike: rationals, the section 13 nominal rate, and the
//! section 5 rule 7 source frame selection. All comparisons are in i128.

use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Rational {
    pub num: i64,
    pub den: i64,
}

impl Rational {
    pub const fn new(num: i64, den: i64) -> Self {
        Rational { num, den }
    }
    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }
}

impl std::fmt::Display for Rational {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}/{}", self.num, self.den)
    }
}

/// Section 5: frame rates the engine supports.
pub const SUPPORTED_RATES: [Rational; 16] = [
    Rational::new(24000, 1001),
    Rational::new(24, 1),
    Rational::new(25, 1),
    Rational::new(30000, 1001),
    Rational::new(30, 1),
    Rational::new(48000, 1001),
    Rational::new(48, 1),
    Rational::new(50, 1),
    Rational::new(60000, 1001),
    Rational::new(60, 1),
    Rational::new(90, 1),
    Rational::new(100, 1),
    Rational::new(120000, 1001),
    Rational::new(120, 1),
    Rational::new(240000, 1001),
    Rational::new(240, 1),
];

#[derive(Clone, Debug, Serialize)]
pub struct NominalRate {
    pub rate: Rational,
    /// "median" when exactly one supported rate is within ±1 timebase unit of the
    /// median duration; "median+mean" when several are and the window mean
    /// decided; "fallback" for (last − first) / (frames − 1).
    pub method: &'static str,
    /// Supported rates within ±1 unit of the median (more than one means the
    /// plan's rule alone is ambiguous for this timebase).
    pub candidates: Vec<Rational>,
    pub median_duration: i64,
}

/// Section 13 probe item 5. `pts` is presentation-ordered, in `tb` units.
///
/// The plan's rule ("the supported rate whose frame duration is within ±1 timebase
/// unit of the median duration") matches both 24 and 23.976 at a 1 ms or 1/600
/// timebase (41.708 vs 41.667 ms; 25.025 vs 25.0 units). The spike breaks such ties
/// with the mean duration over the same window and reports that it had to.
pub fn nominal_rate(pts: &[i64], tb: Rational) -> NominalRate {
    let window = &pts[..pts.len().min(300)];
    let mut durs: Vec<i64> = window.windows(2).map(|w| w[1] - w[0]).collect();
    durs.sort_unstable();
    let fallback = || {
        let n = pts.len() as i64;
        let span = (pts[pts.len() - 1] - pts[0]) as i128;
        // rate = (n - 1) / (span * tb) = (n - 1) * tb.den / (span * tb.num)
        let num = ((n - 1) as i128) * tb.den as i128;
        let den = span * tb.num as i128;
        let g = gcd(num, den);
        Rational::new((num / g) as i64, (den / g) as i64)
    };
    if durs.is_empty() {
        return NominalRate { rate: Rational::new(0, 1), method: "fallback", candidates: vec![], median_duration: 0 };
    }
    let median = durs[durs.len() / 2];
    // A rate's frame duration in tb units is (r.den * tb.den) / (r.num * tb.num).
    // |dur - median| <= 1  <=>  |r.den*tb.den - median*r.num*tb.num| <= r.num*tb.num
    let candidates: Vec<Rational> = SUPPORTED_RATES
        .iter()
        .copied()
        .filter(|r| {
            let a = r.den as i128 * tb.den as i128;
            let b = median as i128 * r.num as i128 * tb.num as i128;
            (a - b).abs() <= r.num as i128 * tb.num as i128
        })
        .collect();
    match candidates.len() {
        0 => NominalRate { rate: fallback(), method: "fallback", candidates, median_duration: median },
        1 => NominalRate { rate: candidates[0], method: "median", candidates, median_duration: median },
        _ => {
            // mean duration = span / (n - 1); pick the candidate closest to it.
            let n = (window.len() - 1) as i128;
            let span = (window[window.len() - 1] - window[0]) as i128;
            // |r.den*tb.den/(r.num*tb.num) - span/n|, compared exactly via cross terms.
            let best = *candidates
                .iter()
                .min_by(|x, y| {
                    let dx = dist(**x, tb, span, n);
                    let dy = dist(**y, tb, span, n);
                    // dx = |a_x| / d_x; compare a_x * d_y with a_y * d_x
                    (dx.0 * dy.1).cmp(&(dy.0 * dx.1))
                })
                .unwrap();
            NominalRate { rate: best, method: "median+mean", candidates, median_duration: median }
        }
    }
}

fn dist(r: Rational, tb: Rational, span: i128, n: i128) -> (i128, i128) {
    // dur = r.den*tb.den / (r.num*tb.num); mean = span / n
    let d = r.num as i128 * tb.num as i128 * n;
    let a = (r.den as i128 * tb.den as i128 * n - span * r.num as i128 * tb.num as i128).abs();
    (a, d)
}

pub fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// Section 5 rule 7: index of the last frame with PTS ≤ t + q/2, where q is the
/// stream's timebase unit capped at a quarter of the nominal frame duration.
///
/// `pts` is presentation-ordered in `tb` units. `t` is `t_num / t_den` seconds.
/// Returns None when every frame starts after t + q/2.
pub fn select_frame(pts: &[i64], tb: Rational, rate: Rational, t_num: i128, t_den: i128) -> Option<usize> {
    // q = min(tb, 1 / (4 * rate)) seconds.
    let (q_num, q_den) = {
        let tb_q = (tb.num as i128, tb.den as i128);
        let cap = (rate.den as i128, 4 * rate.num as i128);
        if tb_q.0 * cap.1 <= cap.0 * tb_q.1 { tb_q } else { cap }
    };
    // limit = t + q/2 = (t_num * 2 q_den + q_num * t_den) / (2 t_den q_den)
    let lim_num = t_num * 2 * q_den + q_num * t_den;
    let lim_den = 2 * t_den * q_den;
    // pts_k * tb.num / tb.den <= lim_num / lim_den
    let le = |p: i64| (p as i128) * tb.num as i128 * lim_den <= lim_num * tb.den as i128;
    let idx = pts.partition_point(|&p| le(p));
    idx.checked_sub(1)
}

/// The naive rule (last frame with PTS ≤ t), kept to show what rule 7 fixes.
pub fn select_frame_naive(pts: &[i64], tb: Rational, t_num: i128, t_den: i128) -> Option<usize> {
    let idx = pts.partition_point(|&p| (p as i128) * tb.num as i128 * t_den <= t_num * tb.den as i128);
    idx.checked_sub(1)
}

/// Composition time of frame n at `rate`, offset by the first frame's PTS, as an
/// exact fraction of seconds: t = pts0 * tb + n / rate.
pub fn frame_time(pts0: i64, tb: Rational, rate: Rational, n: i64) -> (i128, i128) {
    // pts0*tb.num/tb.den + n*rate.den/rate.num
    let num = pts0 as i128 * tb.num as i128 * rate.num as i128 + n as i128 * rate.den as i128 * tb.den as i128;
    let den = tb.den as i128 * rate.num as i128;
    (num, den)
}

/// The composition frame each source frame belongs to when the clip's frame 0
/// is its first presented frame: m_k = round((pts_k − pts_0) × tb × rate), with
/// round half up, in i128.
pub fn grid(pts: &[i64], tb: Rational, rate: Rational) -> Vec<i64> {
    let p0 = pts[0] as i128;
    let den = tb.den as i128 * rate.den as i128;
    pts.iter()
        .map(|&p| {
            let num = (p as i128 - p0) * tb.num as i128 * rate.num as i128;
            (2 * num + den).div_euclid(2 * den) as i64
        })
        .collect()
}

/// The source frame composition frame `n` should show: the last frame whose grid
/// slot is at or before n (a hole in the source holds the previous frame).
pub fn expected_frame(grid: &[i64], n: i64) -> Option<usize> {
    grid.partition_point(|&m| m <= n).checked_sub(1)
}

#[derive(Clone, Debug, Serialize)]
pub struct SelectionCheck {
    pub rate: Rational,
    pub frames: usize,
    /// Composition frames from the first presented frame through the last.
    pub composition_frames: usize,
    /// Composition frames with no source frame of their own (the previous one
    /// is held), and source frames that share a slot with their predecessor.
    pub holes: usize,
    pub doubled: usize,
    /// Composition frames N whose rule 7 selection is not the expected frame.
    pub rule7_mismatches: usize,
    pub rule7_first_mismatch: Option<usize>,
    /// Same with the naive PTS ≤ t rule (duplicates and skips on coarse timebases).
    pub naive_mismatches: usize,
}

/// For a source sampled at its nominal rate, every composition frame N must
/// select the source frame on its grid slot (or hold the previous one across a
/// hole in the source).
pub fn check_selection(pts: &[i64], tb: Rational, rate: Rational) -> SelectionCheck {
    let g = grid(pts, tb, rate);
    let last = *g.last().unwrap();
    let mut mism = 0;
    let mut first = None;
    let mut naive = 0;
    let mut holes = 0;
    for n in 0..=last {
        let exp = expected_frame(&g, n);
        if exp.is_some_and(|k| g[k] != n) {
            holes += 1;
        }
        let (tn, td) = frame_time(pts[0], tb, rate, n);
        if select_frame(pts, tb, rate, tn, td) != exp {
            mism += 1;
            first.get_or_insert(n as usize);
        }
        if select_frame_naive(pts, tb, tn, td) != exp {
            naive += 1;
        }
    }
    let doubled = g.windows(2).filter(|w| w[0] == w[1]).count();
    SelectionCheck {
        rate,
        frames: pts.len(),
        composition_frames: (last + 1) as usize,
        holes,
        doubled,
        rule7_mismatches: mism,
        rule7_first_mismatch: first,
        naive_mismatches: naive,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rounded_pts(rate: Rational, tb: Rational, n: usize) -> Vec<i64> {
        // round-half-up of n * rate.den * tb.den / (rate.num * tb.num)
        (0..n as i128)
            .map(|i| {
                let num = i * rate.den as i128 * tb.den as i128;
                let den = rate.num as i128 * tb.num as i128;
                ((2 * num + den) / (2 * den)) as i64
            })
            .collect()
    }

    #[test]
    fn coarse_timebases_select_every_frame_once() {
        for rate in [Rational::new(24000, 1001), Rational::new(30000, 1001)] {
            for tb in [Rational::new(1, 1000), Rational::new(1, 90000), Rational::new(1, 600)] {
                let pts = rounded_pts(rate, tb, 20_000);
                let c = check_selection(&pts, tb, rate);
                assert_eq!(c.rule7_mismatches, 0, "rate {rate} tb {tb}: first {:?}", c.rule7_first_mismatch);
            }
        }
    }

    #[test]
    fn naive_rule_fails_on_coarse_timebases() {
        let rate = Rational::new(30000, 1001);
        let tb = Rational::new(1, 1000);
        let pts = rounded_pts(rate, tb, 3000);
        assert!(check_selection(&pts, tb, rate).naive_mismatches > 0);
    }

    #[test]
    fn nominal_rate_breaks_the_1ms_tie_by_mean() {
        let rate = Rational::new(24000, 1001);
        let tb = Rational::new(1, 1000);
        let pts = rounded_pts(rate, tb, 400);
        let n = nominal_rate(&pts, tb);
        assert_eq!(n.rate, rate);
        assert_eq!(n.method, "median+mean");
        assert!(n.candidates.contains(&Rational::new(24, 1)));
        let tb600 = Rational::new(1, 600);
        let n = nominal_rate(&rounded_pts(Rational::new(30000, 1001), tb600, 400), tb600);
        assert_eq!(n.rate, Rational::new(30000, 1001));
    }

    #[test]
    fn nominal_rate_exact_timebase() {
        let tb = Rational::new(1, 90000);
        let n = nominal_rate(&rounded_pts(Rational::new(30000, 1001), tb, 400), tb);
        assert_eq!((n.rate, n.method), (Rational::new(30000, 1001), "median"));
    }

    #[test]
    fn negative_and_offset_origins() {
        let rate = Rational::new(25, 1);
        let tb = Rational::new(1, 12800);
        let pts: Vec<i64> = (0..100).map(|i| -1024 + i * 512).collect();
        assert_eq!(check_selection(&pts, tb, rate).rule7_mismatches, 0);
        assert_eq!(select_frame(&pts, tb, rate, -1, 1), None);
    }

    #[test]
    fn a_hole_holds_the_previous_frame() {
        // 29.97 at 1/30000 with frame 5 missing (a stream-copy cut can drop a B frame).
        let rate = Rational::new(30000, 1001);
        let tb = Rational::new(1, 30000);
        let pts: Vec<i64> = (0..10).filter(|&i| i != 5).map(|i| -962 + i * 1001).collect();
        let c = check_selection(&pts, tb, rate);
        assert_eq!((c.composition_frames, c.holes, c.doubled, c.rule7_mismatches), (10, 1, 0, 0));
        let g = grid(&pts, tb, rate);
        assert_eq!(expected_frame(&g, 5), Some(4));
        assert_eq!(expected_frame(&g, 6), Some(5));
    }

    #[test]
    fn grid_rounds_coarse_timestamps_to_their_slot() {
        let rate = Rational::new(24000, 1001);
        for tb in [Rational::new(1, 1000), Rational::new(1, 600), Rational::new(1, 90000)] {
            let pts = rounded_pts(rate, tb, 5000);
            assert_eq!(grid(&pts, tb, rate), (0..5000).collect::<Vec<i64>>());
        }
    }
}
