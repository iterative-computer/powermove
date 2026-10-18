//! `SkDashPathEffect` applied on the CPU before tessellation
//! (`Device::drawGeometryWithPathEffect`): `SkDashPath::InternalFilter` over
//! `SkContourMeasure` (Skia's recursive chord subdivision at the
//! `resScale`-adjusted tolerance, conics evaluated at the subdivision
//! parameters), each dash the measured contour's segment chopped as
//! `SkContourMeasure::getSegment` chops it.
//!
//! Portions are derived from Skia, Copyright Google LLC, used under the BSD
//! 3-Clause license.

use super::skpath::{P, SkPath, Verb};
use crate::render::svg::SvgError;

const CHEAP_DIST_LIMIT: f32 = 0.5;
const MAX_T_VALUE: f32 = 0x3FFF_FFFF as f32;

/// `SkDashPath::InternalFilter` for intervals and a phase at `res_scale`.
pub fn dash_path(path: &SkPath, intervals: &[f32], phase: f32, res_scale: f32) -> Result<SkPath, SvgError> {
    if intervals.len() < 2 || !intervals.len().is_multiple_of(2) {
        return Ok(path.clone());
    }
    let mut length = 0.0f32;
    for v in intervals {
        if *v < 0.0 || !v.is_finite() {
            return Ok(path.clone());
        }
        length += *v;
    }
    if length <= 0.0 {
        return Ok(path.clone());
    }
    // CalcDashParameters.
    let mut phase = phase;
    if phase < 0.0 {
        phase = -phase;
        if phase > length {
            phase = libm::fmodf(phase, length);
        }
        phase = length - phase;
        if phase == length {
            phase = 0.0;
        }
    } else if phase >= length {
        phase = libm::fmodf(phase, length);
    }
    let (initial_index, initial_len) = {
        let mut p = phase;
        let mut i = 0usize;
        loop {
            let gap = intervals[i];
            if p > gap || (p == gap && gap != 0.0) {
                p -= gap;
                i += 1;
                if i == intervals.len() {
                    i = 0;
                }
            } else {
                break (i, gap - p);
            }
        }
    };
    let mut out = SkPath::new();
    let contours = measure_contours(path, res_scale);
    if contours.is_empty() {
        return Ok(out);
    }
    for contour in &contours {
        let mut distance = 0.0f64;
        let mut d_len = f64::from(initial_len);
        let mut index = initial_index;
        let mut skip_first = contour.closed;
        let mut added = false;
        let len = f64::from(contour.length);
        while distance < len {
            added = false;
            if index % 2 == 0 && !skip_first {
                added = true;
                contour.segment(distance as f32, (distance + d_len) as f32, true, &mut out);
            }
            distance += d_len;
            skip_first = false;
            index += 1;
            if index == intervals.len() {
                index = 0;
            }
            d_len = f64::from(intervals[index]);
        }
        if contour.closed && initial_index % 2 == 0 && initial_len >= 0.0 {
            contour.segment(0.0, initial_len, !added, &mut out);
        }
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy)]
enum Curve {
    Line([P; 2]),
    Quad([P; 3]),
    Conic([P; 3], f32),
    Cubic([P; 4]),
}

#[derive(Debug, Clone, Copy)]
struct Segment {
    distance: f32,
    curve: usize,
    t_value: f32,
}

struct Contour {
    curves: Vec<Curve>,
    segs: Vec<Segment>,
    length: f32,
    closed: bool,
}

#[inline]
fn dist(a: P, b: P) -> f32 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    (dx * dx + dy * dy).sqrt()
}

#[inline]
fn t_span_big_enough(t_span: f32) -> bool {
    // (tspan >> 10) != 0 on the 30-bit fixed-point t.
    t_span * MAX_T_VALUE >= 1024.0
}

fn cheap_dist_exceeds_limit(p: P, x: f32, y: f32, tolerance: f32) -> bool {
    (x - p[0]).abs().max((y - p[1]).abs()) > tolerance
}

fn quad_too_curvy(p: &[P; 3], tolerance: f32) -> bool {
    let dx = (p[1][0] * 2.0 - p[0][0] - p[2][0]) * 0.25;
    let dy = (p[1][1] * 2.0 - p[0][1] - p[2][1]) * 0.25;
    (dx * dx + dy * dy).sqrt() > tolerance
}

fn conic_too_curvy(first: P, mid: P, last: P, tolerance: f32) -> bool {
    let midx = (first[0] + last[0]) * 0.5;
    let midy = (first[1] + last[1]) * 0.5;
    cheap_dist_exceeds_limit(mid, midx, midy, tolerance)
}

fn cubic_too_curvy(p: &[P; 4], tolerance: f32) -> bool {
    cheap_dist_exceeds_limit(p[1], (p[0][0] * 2.0 + p[3][0]) / 3.0, (p[0][1] * 2.0 + p[3][1]) / 3.0, tolerance)
        || cheap_dist_exceeds_limit(p[2], (p[0][0] + p[3][0] * 2.0) / 3.0, (p[0][1] + p[3][1] * 2.0) / 3.0, tolerance)
}

#[inline]
fn interp(a: P, b: P, t: f32) -> P {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
}

/// `SkChopQuadAt`.
fn chop_quad_at(p: &[P; 3], t: f32) -> [P; 5] {
    let p01 = interp(p[0], p[1], t);
    let p12 = interp(p[1], p[2], t);
    [p[0], p01, interp(p01, p12, t), p12, p[2]]
}

/// `SkChopCubicAt`.
fn chop_cubic_at(p: &[P; 4], t: f32) -> [P; 7] {
    let ab = interp(p[0], p[1], t);
    let bc = interp(p[1], p[2], t);
    let cd = interp(p[2], p[3], t);
    let abc = interp(ab, bc, t);
    let bcd = interp(bc, cd, t);
    [p[0], ab, abc, interp(abc, bcd, t), bcd, cd, p[3]]
}

/// `SkConic::chopAt(t)`: two conics in standard form.
fn chop_conic_at(p: &[P; 3], w: f32, t: f32) -> ([P; 3], f32, [P; 3], f32) {
    let tmp = [[p[0][0], p[0][1], 1.0f32], [p[1][0] * w, p[1][1] * w, w], [p[2][0], p[2][1], 1.0]];
    let lerp3 = |a: [f32; 3], b: [f32; 3]| [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t];
    let t0 = lerp3(tmp[0], tmp[1]);
    let t2 = lerp3(tmp[1], tmp[2]);
    let t1 = lerp3(t0, t2);
    let project = |v: [f32; 3]| [v[0] / v[2], v[1] / v[2]];
    let root = t1[2].sqrt();
    ([p[0], project(t0), project(t1)], t0[2] / root, [project(t1), project(t2), p[2]], t2[2] / root)
}

fn eval_conic(p: &[P; 3], w: f32, t: f32) -> P {
    // SkConic::evalAt: ratquad in homogeneous coordinates.
    let mt = 1.0 - t;
    let a = mt * mt;
    let b = 2.0 * mt * t;
    let c = t * t;
    let num = |i: usize| a * p[0][i] + b * w * p[1][i] + c * p[2][i];
    let den = a + b * w + c;
    [num(0) / den, num(1) / den]
}

struct Measurer {
    tolerance: f32,
    curves: Vec<Curve>,
    segs: Vec<Segment>,
    length: f32,
}

impl Measurer {
    fn line(&mut self, a: P, b: P) {
        let d = dist(a, b);
        let prev = self.length;
        let next = prev + d;
        if prev < next {
            self.curves.push(Curve::Line([a, b]));
            self.segs.push(Segment { distance: next, curve: self.curves.len() - 1, t_value: 1.0 });
            self.length = next;
        }
    }

    fn quad_segs(&mut self, p: &[P; 3], distance: f32, mint: f32, maxt: f32, idx: usize) -> f32 {
        if t_span_big_enough(maxt - mint) && quad_too_curvy(p, self.tolerance) {
            let tmp = chop_quad_at(p, 0.5);
            let halft = (mint + maxt) * 0.5;
            let d = self.quad_segs(&[tmp[0], tmp[1], tmp[2]], distance, mint, halft, idx);
            self.quad_segs(&[tmp[2], tmp[3], tmp[4]], d, halft, maxt, idx)
        } else {
            let d = dist(p[0], p[2]);
            let prev = distance;
            let distance = distance + d;
            if distance > prev {
                self.segs.push(Segment { distance, curve: idx, t_value: maxt });
            }
            distance
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn conic_segs(&mut self, p: &[P; 3], w: f32, distance: f32, mint: f32, min_pt: P, maxt: f32, max_pt: P, idx: usize) -> f32 {
        let halft = (mint + maxt) * 0.5;
        let half_pt = eval_conic(p, w, halft);
        if !half_pt[0].is_finite() || !half_pt[1].is_finite() {
            return distance;
        }
        if t_span_big_enough(maxt - mint) && conic_too_curvy(min_pt, half_pt, max_pt, self.tolerance) {
            let d = self.conic_segs(p, w, distance, mint, min_pt, halft, half_pt, idx);
            self.conic_segs(p, w, d, halft, half_pt, maxt, max_pt, idx)
        } else {
            let d = dist(min_pt, max_pt);
            let prev = distance;
            let distance = distance + d;
            if distance > prev {
                self.segs.push(Segment { distance, curve: idx, t_value: maxt });
            }
            distance
        }
    }

    fn cubic_segs(&mut self, p: &[P; 4], distance: f32, mint: f32, maxt: f32, idx: usize) -> f32 {
        if t_span_big_enough(maxt - mint) && cubic_too_curvy(p, self.tolerance) {
            let tmp = chop_cubic_at(p, 0.5);
            let halft = (mint + maxt) * 0.5;
            let d = self.cubic_segs(&[tmp[0], tmp[1], tmp[2], tmp[3]], distance, mint, halft, idx);
            self.cubic_segs(&[tmp[3], tmp[4], tmp[5], tmp[6]], d, halft, maxt, idx)
        } else {
            let d = dist(p[0], p[3]);
            let prev = distance;
            let distance = distance + d;
            if distance > prev {
                self.segs.push(Segment { distance, curve: idx, t_value: maxt });
            }
            distance
        }
    }

    fn take(&mut self, closed: bool) -> Option<Contour> {
        let out = (self.length > 0.0).then(|| Contour {
            curves: std::mem::take(&mut self.curves),
            segs: std::mem::take(&mut self.segs),
            length: self.length,
            closed,
        });
        self.curves.clear();
        self.segs.clear();
        self.length = 0.0;
        out
    }
}

/// `SkContourMeasureIter`: every contour with positive length.
fn measure_contours(path: &SkPath, res_scale: f32) -> Vec<Contour> {
    let mut m = Measurer { tolerance: CHEAP_DIST_LIMIT * (1.0 / res_scale), curves: Vec::new(), segs: Vec::new(), length: 0.0 };
    let mut out = Vec::new();
    let mut closed = false;
    for (verb, pts) in path.segments() {
        match verb {
            Verb::Move => {
                out.extend(m.take(closed));
                closed = false;
            }
            Verb::Line => m.line(pts[0], pts[1]),
            Verb::Quad => {
                let p = [pts[0], pts[1], pts[2]];
                m.curves.push(Curve::Quad(p));
                let idx = m.curves.len() - 1;
                let before = m.segs.len();
                let d = m.length;
                let next = m.quad_segs(&p, d, 0.0, 1.0, idx);
                if m.segs.len() == before {
                    m.curves.pop();
                }
                m.length = next;
            }
            Verb::Conic(w) => {
                let p = [pts[0], pts[1], pts[2]];
                m.curves.push(Curve::Conic(p, w));
                let idx = m.curves.len() - 1;
                let before = m.segs.len();
                let d = m.length;
                let next = m.conic_segs(&p, w, d, 0.0, p[0], 1.0, p[2], idx);
                if m.segs.len() == before {
                    m.curves.pop();
                }
                m.length = next;
            }
            Verb::Cubic => {
                let p = [pts[0], pts[1], pts[2], pts[3]];
                m.curves.push(Curve::Cubic(p));
                let idx = m.curves.len() - 1;
                let before = m.segs.len();
                let d = m.length;
                let next = m.cubic_segs(&p, d, 0.0, 1.0, idx);
                if m.segs.len() == before {
                    m.curves.pop();
                }
                m.length = next;
            }
            Verb::Close => {
                if pts[0] != pts[1] {
                    m.line(pts[0], pts[1]);
                }
                closed = true;
            }
        }
    }
    out.extend(m.take(closed));
    out
}

impl Contour {
    /// `distanceToSegment`: the segment index and its parameter at `distance`.
    fn distance_to_segment(&self, distance: f32) -> (usize, f32) {
        let mut idx = self.segs.partition_point(|s| s.distance < distance);
        if idx >= self.segs.len() {
            idx = self.segs.len() - 1;
        }
        let seg = self.segs[idx];
        let (start_t, start_d) = if idx > 0 && self.segs[idx - 1].curve == seg.curve {
            (self.segs[idx - 1].t_value, self.segs[idx - 1].distance)
        } else if idx > 0 {
            (0.0, self.segs[idx - 1].distance)
        } else {
            (0.0, 0.0)
        };
        let span = seg.distance - start_d;
        let t = if span > 0.0 { start_t + (seg.t_value - start_t) * ((distance - start_d) / span) } else { start_t };
        (idx, t)
    }

    /// `getSegment(startD, stopD, dst, startWithMoveTo)`.
    fn segment(&self, start_d: f32, stop_d: f32, move_to: bool, dst: &mut SkPath) {
        let start_d = start_d.max(0.0);
        let stop_d = stop_d.min(self.length);
        // NaN-aware: `!(a <= b)` also catches unordered inputs.
        #[allow(clippy::neg_cmp_op_on_partial_ord)]
        if !(start_d <= stop_d) || start_d == stop_d {
            return;
        }
        let (si, st) = self.distance_to_segment(start_d);
        let (ei, et) = self.distance_to_segment(stop_d);
        let mut first = move_to;
        let mut i = si;
        loop {
            let curve = self.segs[i].curve;
            // Merge consecutive segments of one curve.
            let mut j = i;
            while j < ei && self.segs[j + 1].curve == curve {
                j += 1;
            }
            let t0 = if i == si { st } else { 0.0 };
            let t1 = if j == ei { et } else { 1.0 };
            self.seg_to(curve, t0, t1, first, dst);
            first = false;
            if j >= ei {
                break;
            }
            i = j + 1;
        }
    }

    /// `segTo(pts, type, startT, stopT, dst)`.
    fn seg_to(&self, curve: usize, start_t: f32, stop_t: f32, move_to: bool, dst: &mut SkPath) {
        if start_t == stop_t {
            // A zero-length segment contributes a lone point (Skia emits a
            // lineTo when not starting with moveTo).
            let p = match self.curves[curve] {
                Curve::Line(l) => interp(l[0], l[1], start_t),
                Curve::Quad(p) => chop_quad_at(&p, start_t)[2],
                Curve::Conic(p, w) => eval_conic(&p, w, start_t),
                Curve::Cubic(p) => chop_cubic_at(&p, start_t)[3],
            };
            if move_to {
                dst.move_to(p);
            } else {
                dst.line_to(p);
            }
            return;
        }
        match self.curves[curve] {
            Curve::Line(l) => {
                if move_to {
                    dst.move_to(interp(l[0], l[1], start_t));
                }
                dst.line_to(interp(l[0], l[1], stop_t));
            }
            Curve::Quad(p) => {
                let seg: [P; 3] = if start_t == 0.0 {
                    if stop_t == 1.0 {
                        p
                    } else {
                        let t = chop_quad_at(&p, stop_t);
                        [t[0], t[1], t[2]]
                    }
                } else {
                    let t0 = chop_quad_at(&p, start_t);
                    let right = [t0[2], t0[3], t0[4]];
                    if stop_t == 1.0 {
                        right
                    } else {
                        let t1 = chop_quad_at(&right, (stop_t - start_t) / (1.0 - start_t));
                        [t1[0], t1[1], t1[2]]
                    }
                };
                if move_to {
                    dst.move_to(seg[0]);
                }
                dst.quad_to(seg[1], seg[2]);
            }
            Curve::Conic(p, w) => {
                let (seg, nw): ([P; 3], f32) = if start_t == 0.0 {
                    if stop_t == 1.0 {
                        (p, w)
                    } else {
                        let (l, lw, _, _) = chop_conic_at(&p, w, stop_t);
                        (l, lw)
                    }
                } else {
                    let (_, _, r, rw) = chop_conic_at(&p, w, start_t);
                    if stop_t == 1.0 {
                        (r, rw)
                    } else {
                        let (l, lw, _, _) = chop_conic_at(&r, rw, (stop_t - start_t) / (1.0 - start_t));
                        (l, lw)
                    }
                };
                if move_to {
                    dst.move_to(seg[0]);
                }
                dst.conic_to(seg[1], seg[2], nw);
            }
            Curve::Cubic(p) => {
                let seg: [P; 4] = if start_t == 0.0 {
                    if stop_t == 1.0 {
                        p
                    } else {
                        let t = chop_cubic_at(&p, stop_t);
                        [t[0], t[1], t[2], t[3]]
                    }
                } else {
                    let t0 = chop_cubic_at(&p, start_t);
                    let right = [t0[3], t0[4], t0[5], t0[6]];
                    if stop_t == 1.0 {
                        right
                    } else {
                        let t1 = chop_cubic_at(&right, (stop_t - start_t) / (1.0 - start_t));
                        [t1[0], t1[1], t1[2], t1[3]]
                    }
                };
                if move_to {
                    dst.move_to(seg[0]);
                }
                dst.cubic_to(seg[1], seg[2], seg[3]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashes_a_line_from_the_phase() {
        let mut p = SkPath::new();
        p.move_to([0.0, 0.0]);
        p.line_to([30.0, 0.0]);
        let d = dash_path(&p, &[8.0, 4.0], 0.0, 1.0).unwrap();
        // Dashes at [0,8], [12,20], [24,30].
        let moves = d.verbs.iter().filter(|v| **v == Verb::Move).count();
        assert_eq!(moves, 3);
        assert_eq!(d.pts[0], [0.0, 0.0]);
        assert_eq!(d.pts[1], [8.0, 0.0]);
        assert_eq!(d.pts[2], [12.0, 0.0]);
        assert_eq!(*d.pts.last().unwrap(), [30.0, 0.0]);
    }

    #[test]
    fn closed_contours_join_the_first_dash() {
        let mut p = SkPath::new();
        p.move_to([0.0, 0.0]);
        p.line_to([10.0, 0.0]);
        p.line_to([10.0, 10.0]);
        p.line_to([0.0, 10.0]);
        p.close();
        let d = dash_path(&p, &[6.0, 2.0], 0.0, 1.0).unwrap();
        // The dash that wraps the start is joined to the last one.
        assert!(d.verbs.len() > 4);
        assert_eq!(*d.pts.last().unwrap(), [6.0, 0.0]);
    }
}
