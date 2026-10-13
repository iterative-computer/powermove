//! Exact source time: the nominal rate (section 13 probe item 5, with the
//! section 26 amendment 2 tie-break) and source frame selection (section 5
//! rule 7). Every comparison is exact; nothing goes through floats.
//!
//! Media timestamps stay in their stream's timebase (`pts` units of
//! `timebase` seconds); engine time is [`Tick`]s. Comparisons multiply out in
//! `i128` and widen to 256 bits when a product could overflow.

use pm_time::{Rate, Rational, SUPPORTED_RATES, TICKS_PER_SECOND, Tick};
use serde::Serialize;

/// How [`nominal_rate`] decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RateMethod {
    /// Exactly one supported rate is within ±1 timebase unit of the median
    /// frame duration.
    Median,
    /// Several supported rates qualify; the one closest to the mean frame
    /// duration over the same window won (section 26 amendment 2).
    MedianThenMean,
    /// No supported rate qualifies: (last PTS − first PTS) / (frames − 1).
    Fallback,
}

/// The section 13 nominal rate of a stream.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NominalRate {
    #[serde(serialize_with = "rate_serde::one")]
    pub rate: Rate,
    pub method: RateMethod,
    /// Supported rates within ±1 timebase unit of the median duration.
    #[serde(serialize_with = "rate_serde::many")]
    pub candidates: Vec<Rate>,
    /// Median frame duration over the window, in timebase units.
    pub median_duration: i64,
    /// Frames in the window (at most 300).
    pub window: usize,
}

/// Rates serialize as `[num, den]`.
pub mod rate_serde {
    use pm_time::Rate;
    use serde::Serializer;
    use serde::ser::SerializeSeq;

    pub fn one<S: Serializer>(r: &Rate, s: S) -> Result<S::Ok, S::Error> {
        let mut q = s.serialize_seq(Some(2))?;
        q.serialize_element(&r.num())?;
        q.serialize_element(&r.den())?;
        q.end()
    }

    pub fn many<S: Serializer>(v: &[Rate], s: S) -> Result<S::Ok, S::Error> {
        let mut q = s.serialize_seq(Some(v.len()))?;
        for r in v {
            q.serialize_element(&[r.num(), r.den()])?;
        }
        q.end()
    }
}

/// Frames the nominal rate looks at (section 13 probe item 5).
pub const NOMINAL_RATE_WINDOW: usize = 300;

/// Section 13 probe item 5 with amendment 2. `pts` holds presented frames in
/// presentation order, in `timebase` units. Returns `None` for fewer than two
/// frames or a zero span.
pub fn nominal_rate(pts: &[i64], timebase: Rational) -> Option<NominalRate> {
    if pts.len() < 2 || timebase.num() <= 0 {
        return None;
    }
    let window = &pts[..pts.len().min(NOMINAL_RATE_WINDOW)];
    let mut durations: Vec<i64> = window.windows(2).map(|w| w[1] - w[0]).collect();
    durations.sort_unstable();
    let median = durations[durations.len() / 2];
    let (tn, td) = (timebase.num() as i128, timebase.den() as i128);
    // A rate r has a frame duration of (r.den · tb.den) / (r.num · tb.num) units;
    // |duration − median| ≤ 1  ⇔  |r.den·tb.den − median·r.num·tb.num| ≤ r.num·tb.num.
    let candidates: Vec<Rate> = SUPPORTED_RATES
        .iter()
        .copied()
        .filter(|r| {
            let a = r.den() as i128 * td;
            let b = median as i128 * r.num() as i128 * tn;
            (a - b).abs() <= r.num() as i128 * tn
        })
        .collect();
    let span = (window[window.len() - 1] - window[0]) as i128;
    let intervals = (window.len() - 1) as i128;
    let (rate, method) = match candidates.len() {
        0 => {
            let full_span = (pts[pts.len() - 1] - pts[0]) as i128;
            if full_span <= 0 {
                return None;
            }
            // rate = (n − 1) / (span · tb) = (n − 1)·tb.den / (span·tb.num)
            let num = (pts.len() as i128 - 1) * td;
            let den = full_span * tn;
            let g = gcd(num, den);
            let rate = Rate::new(i64::try_from(num / g).ok()?, i64::try_from(den / g).ok()?).ok()?;
            (rate, RateMethod::Fallback)
        }
        1 => (candidates[0], RateMethod::Median),
        _ => {
            if span <= 0 {
                return None;
            }
            // |duration_r − span/intervals| as an exact fraction a/d, compared by cross products.
            let distance = |r: &Rate| {
                let d = r.num() as i128 * tn * intervals;
                let a = (r.den() as i128 * td * intervals - span * r.num() as i128 * tn).abs();
                (a, d)
            };
            let best = *candidates
                .iter()
                .min_by(|x, y| {
                    let (ax, dx) = distance(x);
                    let (ay, dy) = distance(y);
                    (ax * dy).cmp(&(ay * dx))
                })
                .expect("at least two candidates");
            (best, RateMethod::MedianThenMean)
        }
    };
    Some(NominalRate { rate, method, candidates, median_duration: median, window: window.len() })
}

pub(crate) fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

/// Compares `a_num / a_den` with `b_num / b_den` (positive denominators)
/// exactly, widening to 256 bits when the cross products overflow `i128`.
pub(crate) fn cmp_frac(a_num: i128, a_den: i128, b_num: i128, b_den: i128) -> std::cmp::Ordering {
    debug_assert!(a_den > 0 && b_den > 0);
    match (a_num.checked_mul(b_den), b_num.checked_mul(a_den)) {
        (Some(x), Some(y)) => x.cmp(&y),
        _ => wide_mul(a_num, b_den).cmp(&wide_mul(b_num, a_den)),
    }
}

/// Exact signed 256-bit product as (high, low) words, ordered like the value.
fn wide_mul(a: i128, b: i128) -> (i128, u128) {
    let negative = (a < 0) != (b < 0) && a != 0 && b != 0;
    let (ua, ub) = (a.unsigned_abs(), b.unsigned_abs());
    let (a1, a0) = (ua >> 64, ua & u64::MAX as u128);
    let (b1, b0) = (ub >> 64, ub & u64::MAX as u128);
    let p00 = a0 * b0;
    let p01 = a0 * b1;
    let p10 = a1 * b0;
    let p11 = a1 * b1;
    let mid = (p00 >> 64) + (p01 & u64::MAX as u128) + (p10 & u64::MAX as u128);
    let lo = (p00 & u64::MAX as u128) | (mid << 64);
    let hi = p11 + (p01 >> 64) + (p10 >> 64) + (mid >> 64);
    if !negative {
        (hi as i128, lo)
    } else {
        // Two's complement negation of the 256-bit magnitude.
        let (nlo, carry) = (!lo).overflowing_add(1);
        let nhi = (!hi).wrapping_add(carry as u128);
        (nhi as i128, nlo)
    }
}

/// The section 5 rule 7 quantum q: the stream's timebase unit, capped at a
/// quarter of the nominal frame duration, as seconds (`num / den`).
pub fn selection_quantum(timebase: Rational, rate: Rate) -> (i128, i128) {
    let tb = (timebase.num() as i128, timebase.den() as i128);
    let cap = (rate.den() as i128, 4 * rate.num() as i128);
    if cmp_frac(tb.0, tb.1, cap.0, cap.1).is_le() { tb } else { cap }
}

/// Section 5 rule 7: the index of the last frame with PTS ≤ t + q/2, where q
/// is [`selection_quantum`]. `pts` is presentation-ordered, in `timebase`
/// units; `t` is source time in ticks on the same origin as the PTS (PTS 0 is
/// tick 0). `None` when every frame starts after t + q/2.
pub fn select_frame(pts: &[i64], timebase: Rational, rate: Rate, t: Tick) -> Option<usize> {
    let (q_num, q_den) = selection_quantum(timebase, rate);
    // limit = t/TICKS + q/2 = (2·t·q_den + q_num·TICKS) / (2·TICKS·q_den)
    let lim_num = 2 * t as i128 * q_den + q_num * TICKS_PER_SECOND as i128;
    let lim_den = 2 * TICKS_PER_SECOND as i128 * q_den;
    let (tn, td) = (timebase.num() as i128, timebase.den() as i128);
    let idx = pts.partition_point(|&p| cmp_frac(p as i128 * tn, td, lim_num, lim_den).is_le());
    idx.checked_sub(1)
}

/// The naive rule (last frame with PTS ≤ t), kept for tests that show what
/// rule 7 fixes on coarse timebases.
pub fn select_frame_naive(pts: &[i64], timebase: Rational, t: Tick) -> Option<usize> {
    let (tn, td) = (timebase.num() as i128, timebase.den() as i128);
    let idx = pts
        .partition_point(|&p| cmp_frac(p as i128 * tn, td, t as i128, TICKS_PER_SECOND as i128).is_le());
    idx.checked_sub(1)
}

/// Ticks of a timestamp with round-half-even (exact when the timebase
/// divides the flick). Saturates far outside the representable range.
pub fn ticks_of(pts: i64, timebase: Rational) -> Tick {
    pm_time::ticks_from_timebase(pts, timebase).unwrap_or(if pts < 0 { Tick::MIN } else { Tick::MAX })
}

/// The composition slot of each source frame when the clip's frame 0 is its
/// first presented frame: m_k = round((pts_k − pts_0) × tb × rate), half up.
/// Used to state the expected frame for each composition frame independently
/// of rule 7 (tests and verification).
pub fn grid(pts: &[i64], timebase: Rational, rate: Rate) -> Vec<i64> {
    let Some(&p0) = pts.first() else { return vec![] };
    let den = timebase.den() as i128 * rate.den() as i128;
    pts.iter()
        .map(|&p| {
            let num = (p as i128 - p0 as i128) * timebase.num() as i128 * rate.num() as i128;
            (2 * num + den).div_euclid(2 * den) as i64
        })
        .collect()
}

/// The source frame composition frame `n` should show on `grid`: the last
/// frame whose slot is at or before n (a hole holds the previous frame).
pub fn expected_frame(grid: &[i64], n: i64) -> Option<usize> {
    grid.partition_point(|&m| m <= n).checked_sub(1)
}

/// Source time in ticks of composition frame `n` when the clip's frame 0 is
/// the frame at `pts0`: pts0·tb + start of frame n at `rate` (exact rounding
/// up to the first tick of the frame, as `pm_time::Rate::frame_start`).
pub fn frame_time(pts0: i64, timebase: Rational, rate: Rate, n: i64) -> Tick {
    let origin = (pts0 as i128 * timebase.num() as i128 * TICKS_PER_SECOND as i128)
        .div_euclid(timebase.den() as i128);
    let start = rate.frame_start(n).unwrap_or(if n < 0 { Tick::MIN } else { Tick::MAX });
    (origin + start as i128).clamp(Tick::MIN as i128, Tick::MAX as i128) as Tick
}

/// Rule 7 checked against [`grid`] for every composition frame of a stream.
#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionCheck {
    pub frames: usize,
    pub composition_frames: usize,
    pub holes: usize,
    pub doubled: usize,
    pub rule7_mismatches: usize,
    pub first_mismatch: Option<i64>,
    pub naive_mismatches: usize,
}

/// Every composition frame N must select the source frame on its grid slot
/// (or hold the previous one across a hole in the source).
pub fn check_selection(pts: &[i64], timebase: Rational, rate: Rate) -> SelectionCheck {
    let g = grid(pts, timebase, rate);
    let Some(&last) = g.last() else { return SelectionCheck::default() };
    let mut c = SelectionCheck { frames: pts.len(), composition_frames: (last + 1).max(0) as usize, ..Default::default() };
    for n in 0..=last {
        let expected = expected_frame(&g, n);
        if expected.is_some_and(|k| g[k] != n) {
            c.holes += 1;
        }
        let t = frame_time(pts[0], timebase, rate, n);
        if select_frame(pts, timebase, rate, t) != expected {
            c.rule7_mismatches += 1;
            c.first_mismatch.get_or_insert(n);
        }
        if select_frame_naive(pts, timebase, t) != expected {
            c.naive_mismatches += 1;
        }
    }
    c.doubled = g.windows(2).filter(|w| w[0] == w[1]).count();
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: i64, d: i64) -> Rational {
        Rational::new(n, d).unwrap()
    }

    fn rounded_pts(rate: Rate, tb: Rational, n: usize) -> Vec<i64> {
        (0..n as i128)
            .map(|i| {
                let num = i * rate.den() as i128 * tb.den() as i128;
                let den = rate.num() as i128 * tb.num() as i128;
                ((2 * num + den) / (2 * den)) as i64
            })
            .collect()
    }

    #[test]
    fn coarse_timebases_select_every_frame_once() {
        for rate in [Rate::FPS_23_976, Rate::FPS_29_97, Rate::FPS_59_94] {
            for tb in [r(1, 1000), r(1, 90000), r(1, 600)] {
                let pts = rounded_pts(rate, tb, 20_000);
                let c = check_selection(&pts, tb, rate);
                assert_eq!(c.rule7_mismatches, 0, "rate {rate:?} tb {tb:?}: first {:?}", c.first_mismatch);
                assert_eq!(c.composition_frames, 20_000);
            }
        }
    }

    #[test]
    fn naive_rule_fails_on_coarse_timebases() {
        let tb = r(1, 1000);
        let pts = rounded_pts(Rate::FPS_29_97, tb, 3000);
        assert!(check_selection(&pts, tb, Rate::FPS_29_97).naive_mismatches > 0);
    }

    #[test]
    fn nominal_rate_breaks_ties_by_the_mean() {
        let tb = r(1, 1000);
        let n = nominal_rate(&rounded_pts(Rate::FPS_23_976, tb, 400), tb).unwrap();
        assert_eq!((n.rate, n.method), (Rate::FPS_23_976, RateMethod::MedianThenMean));
        assert!(n.candidates.contains(&Rate::FPS_24));
        let n = nominal_rate(&rounded_pts(Rate::FPS_24, tb, 400), tb).unwrap();
        assert_eq!(n.rate, Rate::FPS_24);
        let tb600 = r(1, 600);
        assert_eq!(nominal_rate(&rounded_pts(Rate::FPS_29_97, tb600, 400), tb600).unwrap().rate, Rate::FPS_29_97);
        assert_eq!(nominal_rate(&rounded_pts(Rate::FPS_30, tb600, 400), tb600).unwrap().rate, Rate::FPS_30);
        // Fine timebases where NTSC and integer rates differ by one unit per frame.
        for tb in [r(1, 24000), r(1, 30000), r(1, 15360)] {
            for rate in [Rate::FPS_23_976, Rate::FPS_24, Rate::FPS_29_97, Rate::FPS_30] {
                let n = nominal_rate(&rounded_pts(rate, tb, 400), tb).unwrap();
                assert_eq!(n.rate, rate, "{rate:?} at {tb:?}");
            }
        }
    }

    #[test]
    fn nominal_rate_exact_and_fallback() {
        let tb = r(1, 90000);
        let n = nominal_rate(&rounded_pts(Rate::FPS_29_97, tb, 400), tb).unwrap();
        assert_eq!((n.rate, n.method), (Rate::FPS_29_97, RateMethod::Median));
        // 12.5 fps is not a supported rate: (last − first)/(n − 1).
        let pts: Vec<i64> = (0..50).map(|i| i * 80).collect();
        let n = nominal_rate(&pts, r(1, 1000)).unwrap();
        assert_eq!((n.rate, n.method), (Rate::new(25, 2).unwrap(), RateMethod::Fallback));
        assert!(nominal_rate(&[5], r(1, 1000)).is_none());
        assert!(nominal_rate(&[5, 5], r(1, 1000)).is_none());
    }

    #[test]
    fn negative_and_offset_origins() {
        let tb = r(1, 12800);
        let pts: Vec<i64> = (0..100).map(|i| -1024 + i * 512).collect();
        assert_eq!(check_selection(&pts, tb, Rate::FPS_25).rule7_mismatches, 0);
        assert_eq!(select_frame(&pts, tb, Rate::FPS_25, -ticks_of(2048, tb)), None);
        assert_eq!(select_frame(&pts, tb, Rate::FPS_25, ticks_of(-1024, tb)), Some(0));
    }

    #[test]
    fn a_hole_holds_the_previous_frame() {
        let tb = r(1, 30000);
        let pts: Vec<i64> = (0..10).filter(|&i| i != 5).map(|i| -962 + i * 1001).collect();
        let c = check_selection(&pts, tb, Rate::FPS_29_97);
        assert_eq!((c.composition_frames, c.holes, c.doubled, c.rule7_mismatches), (10, 1, 0, 0));
        let g = grid(&pts, tb, Rate::FPS_29_97);
        assert_eq!(expected_frame(&g, 5), Some(4));
        assert_eq!(expected_frame(&g, 6), Some(5));
    }

    #[test]
    fn quantum_is_capped_at_a_quarter_frame() {
        // 1/600 at 29.97: a unit (1.67 ms) is below a quarter frame (8.3 ms).
        assert_eq!(selection_quantum(r(1, 600), Rate::FPS_29_97), (1, 600));
        // A 1/10 s timebase at 60 fps is coarser than a quarter frame.
        assert_eq!(selection_quantum(r(1, 10), Rate::FPS_60), (1, 240));
    }

    #[test]
    fn wide_comparisons_match_exact_arithmetic() {
        let big = i64::MAX as i128;
        // (big·big)/1 vs (big·big + 1)/1, beyond i128 cross products.
        assert!(cmp_frac(big * big, 3, big * big, 2).is_lt());
        assert!(cmp_frac(-(big * big), 3, big * big, 2).is_lt());
        assert!(cmp_frac(big * big, big, big * big - 1, big).is_gt());
        assert!(cmp_frac(-big * big, 7, -big * big, 7).is_eq());
        // Agreement with i128 where it does not overflow.
        for (a, b, c, d) in [(3, 4, 5, 6), (-3, 4, 5, 6), (7, 1, 7, 1), (-1, 3, -1, 2)] {
            assert_eq!(cmp_frac(a, b, c, d), (a * d).cmp(&(c * b)));
        }
    }

    #[test]
    fn grid_rounds_coarse_timestamps_to_their_slot() {
        for tb in [r(1, 1000), r(1, 600), r(1, 90000)] {
            let pts = rounded_pts(Rate::FPS_23_976, tb, 5000);
            assert_eq!(grid(&pts, tb, Rate::FPS_23_976), (0..5000).collect::<Vec<i64>>());
        }
    }
}
