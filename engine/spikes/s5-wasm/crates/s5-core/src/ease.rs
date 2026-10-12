//! After Effects temporal interpolation, ported from
//! `apps/desktop/src/renderer/src/core/anim/temporal-ease.ts` and the dense
//! evaluation path in `legacy/core/temporal-bridge.ts`.
//!
//! Each side of a key carries a speed (value units per second) and an
//! influence (percent of the adjoining segment). Segment A -> B is the cubic
//! through P0 = (tA, vA), P1 = (tA + fOut*dt, vA + fOut*dt*speedOut),
//! P2 = (tB - fIn*dt, vB - fIn*dt*speedIn), P3 = (tB, vB).
//!
//! Every floating-point expression keeps the operation order of the
//! TypeScript source so results match V8 to the last bit wherever IEEE 754
//! guarantees it (no fused multiply-add, no reassociation).

// Negated comparisons are deliberate: `!(x > 0)` is true for NaN, as in JS.
#![allow(
    clippy::neg_cmp_op_on_partial_ord,
    clippy::manual_clamp,
    clippy::manual_range_contains
)]

/// AE refuses to retract a handle completely; 0.1% is its floor.
pub const MIN_INFLUENCE: f64 = 0.1;
pub const MAX_INFLUENCE: f64 = 100.0;
/// A linear side is the chord's own third: the exact straight line.
pub const LINEAR_INFLUENCE: f64 = 100.0 / 3.0;
/// AE's influence when a key is converted to Auto Bezier.
pub const AUTO_INFLUENCE: f64 = 100.0 / 6.0;
const EPSILON: f64 = 1e-9;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Linear,
    Bezier,
    Hold,
}

impl Interp {
    /// `isInterpolationType` plus the `normalizeKeyEase` fallback to linear.
    pub fn parse(value: Option<&str>) -> Interp {
        match value {
            Some("bezier") => Interp::Bezier,
            Some("hold") => Interp::Hold,
            _ => Interp::Linear,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ease {
    pub speed: f64,
    pub influence: f64,
}

pub fn clamp_influence(value: f64) -> f64 {
    if !value.is_finite() {
        return LINEAR_INFLUENCE;
    }
    if value < MIN_INFLUENCE {
        MIN_INFLUENCE
    } else if value > MAX_INFLUENCE {
        MAX_INFLUENCE
    } else {
        value
    }
}

/// `makeEase`: non-finite speeds become 0, influence is clamped.
pub fn make_ease(speed: f64, influence: f64) -> Ease {
    Ease {
        speed: if speed.is_finite() { speed } else { 0.0 },
        influence: clamp_influence(influence),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Key {
    pub t: f64,
    pub v: f64,
    pub in_interp: Interp,
    pub out_interp: Interp,
    pub in_ease: Ease,
    pub out_ease: Ease,
    pub auto_bezier: bool,
    pub continuous: bool,
}

fn chord_speed(a: &Key, b: &Key) -> f64 {
    let dt = b.t - a.t;
    if !(dt > EPSILON) {
        return 0.0;
    }
    (b.v - a.v) / dt
}

fn resolved_out_ease(a: &Key, b: &Key) -> Ease {
    if a.out_interp != Interp::Bezier {
        return Ease {
            speed: chord_speed(a, b),
            influence: LINEAR_INFLUENCE,
        };
    }
    make_ease(a.out_ease.speed, a.out_ease.influence)
}

fn resolved_in_ease(a: &Key, b: &Key) -> Ease {
    if b.in_interp != Interp::Bezier {
        return Ease {
            speed: chord_speed(a, b),
            influence: LINEAR_INFLUENCE,
        };
    }
    make_ease(b.in_ease.speed, b.in_ease.influence)
}

/// `autoBezierSpeed`: slope through the neighbours, flattened at extrema.
fn auto_bezier_speed(previous: Option<&Key>, key: &Key, next: Option<&Key>) -> f64 {
    let (Some(previous), Some(next)) = (previous, next) else {
        return 0.0;
    };
    let dt = next.t - previous.t;
    if !(dt > EPSILON) {
        return 0.0;
    }
    let before = key.v - previous.v;
    let after = next.v - key.v;
    if before == 0.0 || after == 0.0 || (before > 0.0) != (after > 0.0) {
        return 0.0;
    }
    (next.v - previous.v) / dt
}

/// `refreshAutoBezier`, run when a track is prepared (as `adoptTemporalEase`
/// does before every dense evaluation in the legacy bridge).
pub fn refresh_auto_bezier(keys: &mut [Key]) {
    for index in 0..keys.len() {
        if !keys[index].auto_bezier {
            continue;
        }
        let previous = if index > 0 {
            Some(keys[index - 1])
        } else {
            None
        };
        let next = keys.get(index + 1).copied();
        let speed = auto_bezier_speed(previous.as_ref(), &keys[index], next.as_ref());
        let ease = make_ease(speed, AUTO_INFLUENCE);
        let key = &mut keys[index];
        if key.in_interp != Interp::Hold {
            key.in_interp = Interp::Bezier;
            key.in_ease = ease;
        }
        if key.out_interp != Interp::Hold {
            key.out_interp = Interp::Bezier;
            key.out_ease = ease;
        }
    }
}

/// `solveCurveParam`: Newton with a bisection fallback, constants as in TS.
pub fn solve_curve_param(x1: f64, x2: f64, x: f64) -> f64 {
    if !(x > 0.0) {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let mut u = x;
    for _ in 0..8 {
        let d = ((ax * u + bx) * u + cx) * u - x;
        if d > -1e-7 && d < 1e-7 {
            return u;
        }
        let slope = (3.0 * ax * u + 2.0 * bx) * u + cx;
        if slope > -1e-7 && slope < 1e-7 {
            break;
        }
        let next = u - d / slope;
        if !(next >= 0.0 && next <= 1.0) {
            break;
        }
        u = next;
    }
    let mut lo = 0.0;
    let mut hi = 1.0;
    u = x;
    for _ in 0..32 {
        let d = ((ax * u + bx) * u + cx) * u - x;
        if d > -1e-7 && d < 1e-7 {
            break;
        }
        if d > 0.0 {
            hi = u;
        } else {
            lo = u;
        }
        u = (lo + hi) / 2.0;
    }
    u
}

/// One segment prepared the way `temporalValue` caches it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Hold {
        v: f64,
    },
    Linear {
        start: f64,
        span: f64,
        v: f64,
        d: f64,
    },
    Bezier {
        start: f64,
        span: f64,
        v: f64,
        x1: f64,
        x2: f64,
        ay: f64,
        by: f64,
        cy: f64,
    },
}

impl Curve {
    pub fn new(a: &Key, b: &Key) -> Curve {
        if a.out_interp == Interp::Hold || b.in_interp == Interp::Hold {
            return Curve::Hold { v: a.v };
        }
        let span = b.t - a.t;
        let start = a.t;
        if a.out_interp == Interp::Linear && b.in_interp == Interp::Linear {
            return Curve::Linear {
                start,
                span,
                v: a.v,
                d: b.v - a.v,
            };
        }
        // segmentControlPoints
        let dt = b.t - a.t;
        let out = resolved_out_ease(a, b);
        let incoming = resolved_in_ease(a, b);
        let f_out = out.influence / 100.0;
        let f_in = incoming.influence / 100.0;
        let p1t = a.t + f_out * dt;
        let p1v = a.v + f_out * dt * out.speed;
        let p2t = b.t - f_in * dt;
        let p2v = b.v - f_in * dt * incoming.speed;
        Curve::Bezier {
            start,
            span,
            v: a.v,
            x1: (p1t - start) / span,
            x2: (p2t - start) / span,
            cy: 3.0 * (p1v - a.v),
            by: 3.0 * (p2v - 2.0 * p1v + a.v),
            ay: b.v - 3.0 * p2v + 3.0 * p1v - a.v,
        }
    }

    #[inline]
    pub fn value(&self, time: f64) -> f64 {
        match *self {
            Curve::Hold { v } => v,
            Curve::Linear { start, span, v, d } => v + d * (time - start) / span,
            Curve::Bezier {
                start,
                span,
                v,
                x1,
                x2,
                ay,
                by,
                cy,
            } => {
                let u = solve_curve_param(x1, x2, (time - start) / span);
                ((ay * u + by) * u + cy) * u + v
            }
        }
    }
}

/// A numeric keyframe track with dense times, values and prepared curves.
#[derive(Clone, Debug, PartialEq)]
pub struct Track {
    pub keys: Vec<Key>,
    times: Vec<f64>,
    values: Vec<f64>,
    curves: Vec<Curve>,
}

impl Track {
    /// Keys must already be sorted by time (the document stores them sorted,
    /// `PM.normalizeKeyframes`). Auto-bezier tangents are refreshed here.
    pub fn new(mut keys: Vec<Key>) -> Track {
        refresh_auto_bezier(&mut keys);
        let times = keys.iter().map(|k| k.t).collect();
        let values = keys.iter().map(|k| k.v).collect();
        let curves = keys.windows(2).map(|w| Curve::new(&w[0], &w[1])).collect();
        Track {
            keys,
            times,
            values,
            curves,
        }
    }

    /// `temporalEvaluate` without its single-entry memo (the caller caches).
    pub fn value(&self, time: f64) -> f64 {
        let times = &self.times;
        let n = times.len();
        if n == 0 {
            return f64::NAN;
        }
        // evalKfs: a non-finite time answers the first key.
        if !time.is_finite() {
            return self.values[0];
        }
        if time <= times[0] {
            return self.values[0];
        }
        if time >= times[n - 1] {
            return self.values[n - 1];
        }
        let mut lo = 0usize;
        let mut hi = n - 1;
        while hi - lo > 1 {
            let m = (lo + hi) >> 1;
            if times[m] <= time {
                lo = m;
            } else {
                hi = m;
            }
        }
        self.curves[lo].value(time)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(t: f64, v: f64, out_interp: Interp, in_interp: Interp) -> Key {
        Key {
            t,
            v,
            in_interp,
            out_interp,
            in_ease: make_ease(0.0, LINEAR_INFLUENCE),
            out_ease: make_ease(0.0, LINEAR_INFLUENCE),
            auto_bezier: false,
            continuous: false,
        }
    }

    #[test]
    fn linear_segment_is_exact_lerp() {
        let track = Track::new(vec![
            key(0.0, 0.0, Interp::Linear, Interp::Linear),
            key(2.0, 100.0, Interp::Linear, Interp::Linear),
        ]);
        assert_eq!(track.value(1.0), 50.0);
        assert_eq!(track.value(-1.0), 0.0);
        assert_eq!(track.value(3.0), 100.0);
        assert_eq!(track.value(f64::NAN), 0.0);
    }

    #[test]
    fn hold_freezes_segment() {
        let track = Track::new(vec![
            key(0.0, 10.0, Interp::Hold, Interp::Linear),
            key(1.0, 20.0, Interp::Linear, Interp::Linear),
        ]);
        assert_eq!(track.value(0.999), 10.0);
        assert_eq!(track.value(1.0), 20.0);
    }

    #[test]
    fn bezier_with_chord_speed_on_both_sides_is_the_straight_line() {
        // A bezier side at the chord speed and 33.33% influence is the line.
        let mut a = key(0.0, 0.0, Interp::Bezier, Interp::Linear);
        a.out_ease = make_ease(50.0, LINEAR_INFLUENCE);
        let mut b = key(2.0, 100.0, Interp::Linear, Interp::Bezier);
        b.in_ease = make_ease(50.0, LINEAR_INFLUENCE);
        let track = Track::new(vec![a, b]);
        for i in 1..20 {
            let t = i as f64 / 10.0;
            assert!((track.value(t) - 50.0 * t).abs() < 1e-6, "t={t}");
        }
    }

    #[test]
    fn easy_ease_is_symmetric_and_monotonic() {
        let mut a = key(0.0, 0.0, Interp::Bezier, Interp::Linear);
        a.out_ease = make_ease(0.0, 100.0 / 3.0);
        let mut b = key(1.0, 1.0, Interp::Linear, Interp::Bezier);
        b.in_ease = make_ease(0.0, 100.0 / 3.0);
        let track = Track::new(vec![a, b]);
        assert!((track.value(0.5) - 0.5).abs() < 1e-6);
        let mut previous = 0.0;
        for i in 1..100 {
            let v = track.value(i as f64 / 100.0);
            assert!(v >= previous);
            previous = v;
        }
        assert!(track.value(0.1) < 0.1);
    }

    #[test]
    fn auto_bezier_flattens_at_extrema() {
        let mut keys = vec![
            key(0.0, 0.0, Interp::Linear, Interp::Linear),
            key(1.0, 10.0, Interp::Linear, Interp::Linear),
            key(2.0, 0.0, Interp::Linear, Interp::Linear),
        ];
        keys[1].auto_bezier = true;
        let track = Track::new(keys);
        assert_eq!(track.keys[1].in_interp, Interp::Bezier);
        assert_eq!(track.keys[1].out_ease.speed, 0.0);
        assert_eq!(track.keys[1].out_ease.influence, AUTO_INFLUENCE);
        assert!(track.value(1.0) == 10.0);
        assert!(track.value(0.9) <= 10.0 && track.value(1.1) <= 10.0);
    }

    #[test]
    fn influence_is_clamped() {
        assert_eq!(clamp_influence(0.0), MIN_INFLUENCE);
        assert_eq!(clamp_influence(250.0), MAX_INFLUENCE);
        assert_eq!(clamp_influence(f64::NAN), LINEAR_INFLUENCE);
        assert_eq!(make_ease(f64::INFINITY, 50.0).speed, 0.0);
    }
}
