//! After Effects temporal interpolation, ported from
//! `apps/desktop/src/renderer/src/core/anim/temporal-ease.ts`.
//!
//! Each side of a key carries a *speed* (value units per second) and an
//! *influence* (percent of the adjoining segment). Segment A → B is the cubic
//! through
//!
//! ```text
//! P0 = (tA, vA)
//! P1 = (tA + fOut·dt, vA + fOut·dt·speedOut)
//! P2 = (tB − fIn·dt,  vB − fIn·dt·speedIn)
//! P3 = (tB, vB)
//! ```
//!
//! with `f = influence / 100` and `dt = tB − tA`.
//!
//! These are the typed functions over [`EaseKeyframe`]s, for keys whose time
//! is a number. A key's value is "numeric" (`typeof v === 'number' &&
//! Number.isFinite(v)`) when [`EaseKeyframe::v`] is `Some`. The evaluator's
//! dense path, which also handles raw JSON keys, is in [`crate::keyframes`].
//!
//! Every floating-point expression keeps the TypeScript operation order
//! (left to right, no fused multiply-add), so results match V8 to the bit.
//!
//! The editing commands of the same file (`easyEase`, `setInterpolation`,
//! `setHandleFromPoint`, `applyUnitCurve`, `setHold`, `applyContinuity`) are
//! edits, not evaluation, and belong to `pm-edit`.

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
/// AE's influence when a key is converted to Auto or Continuous Bezier.
pub const AUTO_INFLUENCE: f64 = 100.0 / 6.0;
/// AE's Easy Ease (F9).
pub const EASY_EASE_INFLUENCE: f64 = 100.0 / 3.0;
/// The epsilon of `temporal-ease.ts` (segment lengths and value deltas).
pub const EPSILON: f64 = 1e-9;
/// The convergence window of [`solve_curve_param`] (both the residual and
/// the slope).
pub const SOLVE_EPSILON: f64 = 1e-7;

/// A side's interpolation. [`Interp::Other`] is any stored value that is
/// none of the three (raw keys that were never normalized); it compares
/// unequal to all of them, as the TypeScript string comparisons do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interp {
    Linear,
    Bezier,
    Hold,
    Other,
}

impl Interp {
    /// The interpolation named by a stored string (`Other` for anything else).
    pub fn from_name(name: Option<&[u8]>) -> Interp {
        match name {
            Some(b"linear") => Interp::Linear,
            Some(b"bezier") => Interp::Bezier,
            Some(b"hold") => Interp::Hold,
            _ => Interp::Other,
        }
    }

    /// `isInterpolationType`.
    pub fn is_valid(self) -> bool {
        self != Interp::Other
    }

    /// The name, for valid interpolations.
    pub fn name(self) -> Option<&'static str> {
        match self {
            Interp::Linear => Some("linear"),
            Interp::Bezier => Some("bezier"),
            Interp::Hold => Some("hold"),
            Interp::Other => None,
        }
    }
}

/// Speed (value units per second) and influence (percent).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TemporalEase {
    pub speed: f64,
    pub influence: f64,
}

/// `clampInfluence(Number(value))`: non-finite becomes linear influence,
/// otherwise clamped to 0.1 to 100.
pub fn clamp_influence(number: f64) -> f64 {
    if !number.is_finite() {
        return LINEAR_INFLUENCE;
    }
    if number < MIN_INFLUENCE {
        MIN_INFLUENCE
    } else if number > MAX_INFLUENCE {
        MAX_INFLUENCE
    } else {
        number
    }
}

/// `makeEase(speed, influence)` on already-coerced numbers: a non-finite speed
/// becomes 0.
pub fn make_ease(speed: f64, influence: f64) -> TemporalEase {
    TemporalEase {
        speed: if speed.is_finite() { speed } else { 0.0 },
        influence: clamp_influence(influence),
    }
}

/// One key in the AE model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EaseKeyframe {
    pub t: f64,
    /// The value when it is a finite number.
    pub v: Option<f64>,
    pub in_interp: Interp,
    pub out_interp: Interp,
    pub in_ease: TemporalEase,
    pub out_ease: TemporalEase,
    pub auto_bezier: bool,
    pub continuous: bool,
}

impl EaseKeyframe {
    /// A linear key (`linearKeyframeEase`).
    pub fn linear(t: f64, v: f64) -> EaseKeyframe {
        EaseKeyframe {
            t,
            v: v.is_finite().then_some(v),
            in_interp: Interp::Linear,
            out_interp: Interp::Linear,
            in_ease: make_ease(0.0, LINEAR_INFLUENCE),
            out_ease: make_ease(0.0, LINEAR_INFLUENCE),
            auto_bezier: false,
            continuous: false,
        }
    }
}

/// A point in (seconds, value) space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeValue {
    pub t: f64,
    pub v: f64,
}

/// `chordSpeed(a, b)`: the straight-line speed across a segment.
pub fn chord_speed(a: &EaseKeyframe, b: &EaseKeyframe) -> f64 {
    let dt = b.t - a.t;
    match (a.v, b.v) {
        (Some(va), Some(vb)) if dt > EPSILON => (vb - va) / dt,
        _ => 0.0,
    }
}

/// `resolvedOutEase(a, b)`: `linear` (anything but `bezier`) resolves to the
/// chord.
pub fn resolved_out_ease(a: &EaseKeyframe, b: &EaseKeyframe) -> TemporalEase {
    if a.out_interp != Interp::Bezier {
        return TemporalEase {
            speed: chord_speed(a, b),
            influence: LINEAR_INFLUENCE,
        };
    }
    make_ease(a.out_ease.speed, a.out_ease.influence)
}

/// `resolvedInEase(a, b)`.
pub fn resolved_in_ease(a: &EaseKeyframe, b: &EaseKeyframe) -> TemporalEase {
    if b.in_interp != Interp::Bezier {
        return TemporalEase {
            speed: chord_speed(a, b),
            influence: LINEAR_INFLUENCE,
        };
    }
    make_ease(b.in_ease.speed, b.in_ease.influence)
}

/// The four control points of a segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SegmentCurve {
    pub p0: TimeValue,
    pub p1: TimeValue,
    pub p2: TimeValue,
    pub p3: TimeValue,
}

/// `segmentControlPoints(a, b)`.
pub fn segment_control_points(a: &EaseKeyframe, b: &EaseKeyframe) -> SegmentCurve {
    let dt = b.t - a.t;
    let va = a.v.unwrap_or(0.0);
    let vb = b.v.unwrap_or(va);
    let out = resolved_out_ease(a, b);
    let incoming = resolved_in_ease(a, b);
    let f_out = out.influence / 100.0;
    let f_in = incoming.influence / 100.0;
    SegmentCurve {
        p0: TimeValue { t: a.t, v: va },
        p1: TimeValue {
            t: a.t + f_out * dt,
            v: va + f_out * dt * out.speed,
        },
        p2: TimeValue {
            t: b.t - f_in * dt,
            v: vb - f_in * dt * incoming.speed,
        },
        p3: TimeValue { t: b.t, v: vb },
    }
}

/// The cubic Bernstein form, in the TypeScript's operation order.
pub fn cubic(p0: f64, p1: f64, p2: f64, p3: f64, u: f64) -> f64 {
    let m = 1.0 - u;
    m * m * m * p0 + 3.0 * m * m * u * p1 + 3.0 * m * u * u * p2 + u * u * u * p3
}

/// `solveCurveParam(x1, x2, x)`: the curve parameter where the time
/// component reaches `x`. Newton (8 steps), then bisection (32 steps), with
/// the TypeScript's constants.
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
        if d > -SOLVE_EPSILON && d < SOLVE_EPSILON {
            return u;
        }
        let slope = (3.0 * ax * u + 2.0 * bx) * u + cx;
        if slope > -SOLVE_EPSILON && slope < SOLVE_EPSILON {
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
        if d > -SOLVE_EPSILON && d < SOLVE_EPSILON {
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

/// `segmentParamAtTime(a, b, time)`.
pub fn segment_param_at_time(a: &EaseKeyframe, b: &EaseKeyframe, time: f64) -> f64 {
    let dt = b.t - a.t;
    if !(dt > EPSILON) {
        return 1.0;
    }
    let x = (time - a.t) / dt;
    if !(x > 0.0) {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let curve = segment_control_points(a, b);
    solve_curve_param((curve.p1.t - a.t) / dt, (curve.p2.t - a.t) / dt, x)
}

/// `isHeldSegment(a, b)`.
pub fn is_held_segment(a: &EaseKeyframe, b: &EaseKeyframe) -> bool {
    a.out_interp == Interp::Hold || b.in_interp == Interp::Hold
}

/// `valueAtTime(a, b, time)`: the value of segment A → B, honouring holds.
pub fn value_at_time(a: &EaseKeyframe, b: &EaseKeyframe, time: f64) -> f64 {
    let va = a.v.unwrap_or(0.0);
    if is_held_segment(a, b) {
        return va;
    }
    let Some(vb) = b.v else {
        return va;
    };
    if a.out_interp == Interp::Linear && b.in_interp == Interp::Linear {
        let dt = b.t - a.t;
        if !(dt > EPSILON) {
            return vb;
        }
        let x = (time - a.t) / dt;
        return va
            + (vb - va)
                * (if x < 0.0 {
                    0.0
                } else if x > 1.0 {
                    1.0
                } else {
                    x
                });
    }
    let curve = segment_control_points(a, b);
    let u = segment_param_at_time(a, b, time);
    cubic(curve.p0.v, curve.p1.v, curve.p2.v, curve.p3.v, u)
}

/// Which side of a key a handle belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    In,
    Out,
}

/// `easeFromUnitHandle(handle, dt, dv, side)` on coerced handle numbers.
pub fn ease_from_unit_handle(handle: [f64; 2], dt: f64, dv: f64, side: Side) -> TemporalEase {
    let [x, y] = handle;
    if !x.is_finite() || !y.is_finite() {
        return make_ease(0.0, LINEAR_INFLUENCE);
    }
    let fraction = if side == Side::Out { x } else { 1.0 - x };
    let rise = if side == Side::Out { y } else { 1.0 - y };
    let influence = clamp_influence(fraction * 100.0);
    let span = influence / 100.0 * dt;
    if !(span.abs() > EPSILON) || !dv.is_finite() {
        return make_ease(0.0, influence);
    }
    make_ease(rise * dv / span, influence)
}

/// `unitHandleFromEase(ease, dt, dv, side)`; `None` when the segment is flat.
pub fn unit_handle_from_ease(ease: TemporalEase, dt: f64, dv: f64, side: Side) -> Option<[f64; 2]> {
    if !(dt > EPSILON) || !(dv.abs() > EPSILON) {
        return None;
    }
    let fraction = clamp_influence(ease.influence) / 100.0;
    let rise = fraction * dt * ease.speed / dv;
    Some(if side == Side::Out {
        [fraction, rise]
    } else {
        [1.0 - fraction, 1.0 - rise]
    })
}

/// `segmentUnitCurve(a, b)`: the segment as a CSS `cubic-bezier`; `None`
/// when flat.
pub fn segment_unit_curve(a: &EaseKeyframe, b: &EaseKeyframe) -> Option<[f64; 4]> {
    let dt = b.t - a.t;
    let (Some(va), Some(vb)) = (a.v, b.v) else {
        return None;
    };
    if !(dt > EPSILON) {
        return None;
    }
    let dv = vb - va;
    if !(dv.abs() > EPSILON) {
        return None;
    }
    let curve = segment_control_points(a, b);
    Some([
        (curve.p1.t - a.t) / dt,
        (curve.p1.v - va) / dv,
        (curve.p2.t - a.t) / dt,
        (curve.p2.v - va) / dv,
    ])
}

/// `autoBezierSpeed(previous, key, next)`: the slope through the neighbours,
/// flattened at a local extremum.
pub fn auto_bezier_speed(
    previous: Option<&EaseKeyframe>,
    key: &EaseKeyframe,
    next: Option<&EaseKeyframe>,
) -> f64 {
    let (Some(previous), Some(next)) = (previous, next) else {
        return 0.0;
    };
    let (Some(vp), Some(vk), Some(vn)) = (previous.v, key.v, next.v) else {
        return 0.0;
    };
    let dt = next.t - previous.t;
    if !(dt > EPSILON) {
        return 0.0;
    }
    let before = vk - vp;
    let after = vn - vk;
    if before == 0.0 || after == 0.0 || (before > 0.0) != (after > 0.0) {
        return 0.0;
    }
    (vn - vp) / dt
}

/// `refreshAutoBezier(keys)`: recompute every auto-bezier tangent.
pub fn refresh_auto_bezier(keys: &mut [EaseKeyframe]) {
    for index in 0..keys.len() {
        if !keys[index].auto_bezier {
            continue;
        }
        let previous = index.checked_sub(1).map(|i| keys[i]);
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

/// `handleControlPoint(keys, index, side)`: where a key's direction handle
/// sits, or `None` when hidden.
pub fn handle_control_point(keys: &[EaseKeyframe], index: usize, side: Side) -> Option<TimeValue> {
    let key = keys.get(index)?;
    match side {
        Side::Out => {
            let next = keys.get(index + 1)?;
            if is_held_segment(key, next) {
                return None;
            }
            Some(segment_control_points(key, next).p1)
        }
        Side::In => {
            let previous = keys.get(index.checked_sub(1)?)?;
            if is_held_segment(previous, key) {
                return None;
            }
            Some(segment_control_points(previous, key).p2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn influence_is_clamped() {
        assert_eq!(clamp_influence(0.0), MIN_INFLUENCE);
        assert_eq!(clamp_influence(250.0), MAX_INFLUENCE);
        assert_eq!(clamp_influence(f64::NAN), LINEAR_INFLUENCE);
        assert_eq!(make_ease(f64::INFINITY, 50.0).speed, 0.0);
    }

    #[test]
    fn linear_value_clamps_progress() {
        let a = EaseKeyframe::linear(0.0, 0.0);
        let b = EaseKeyframe::linear(2.0, 100.0);
        assert_eq!(value_at_time(&a, &b, 1.0), 50.0);
        assert_eq!(value_at_time(&a, &b, 3.0), 100.0);
    }

    #[test]
    fn auto_bezier_flattens_at_extrema() {
        let mut keys = [
            EaseKeyframe::linear(0.0, 0.0),
            EaseKeyframe::linear(1.0, 10.0),
            EaseKeyframe::linear(2.0, 0.0),
        ];
        keys[1].auto_bezier = true;
        refresh_auto_bezier(&mut keys);
        assert_eq!(keys[1].in_interp, Interp::Bezier);
        assert_eq!(keys[1].out_ease.speed, 0.0);
        assert_eq!(keys[1].out_ease.influence, AUTO_INFLUENCE);
    }
}
