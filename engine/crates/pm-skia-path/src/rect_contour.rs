//! Skia chrome/m152 (747cf654) `SkPathPriv::IsRectContour` with
//! `allowPartial == false` (src/core/SkPathPriv.cpp:80-253), the recognizer
//! behind `SkPath::isRect` and `SkPathRaw::isRect`: `SkStroke::strokePath`'s
//! rectangle fast path and the analytic scan converter's fat-rectangle
//! blit both use it.
//!
//! Portions are derived from Skia, Copyright Google LLC, used under the BSD
//! 3-Clause license.

use crate::path::PathVerb;
use crate::{Path, Point, Rect};

/// `SkPathPriv::RectContour` (the fields `SkPath::isRect` reports).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectContour {
    /// `fRect`, sorted.
    pub rect: Rect,
    /// `fIsClosed`: the contour ends with a close verb.
    pub closed: bool,
    /// `fDirection == SkPathDirection::kCW` (y down).
    pub clockwise: bool,
}

/// Bit 0: the segment is horizontal; bit 1: it moves right or down. Two
/// directions are opposite iff `a ^ b == 2`.
fn rect_make_dir(dx: f32, dy: f32) -> i32 {
    i32::from(dx != 0.0) | (i32::from(dx > 0.0 || dy > 0.0) << 1)
}

/// `SkRect::MakeLTRB(...).makeSorted()` / `SkRect::set(a, b)`.
fn sorted(a: Point, b: Point) -> Option<Rect> {
    Rect::from_ltrb(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
}

/// `trivial_rect`: exactly move, three lines and a close, axis aligned with
/// three right corners.
fn trivial_rect(pts: &[Point], vbs: &[PathVerb]) -> Option<Option<RectContour>> {
    use PathVerb::{Close, Line, Move};
    if pts.len() != 4 || vbs != [Move, Line, Line, Line, Close] {
        return None;
    }
    let v0 = pts[1] - pts[0];
    let v1 = pts[2] - pts[1];
    let v2 = pts[3] - pts[2];
    let v3 = pts[0] - pts[3];
    // One vertical and one horizontal vector.
    let axis_aligned_orthogonal =
        |a: Point, b: Point| ((a.x == 0.0) ^ (b.x == 0.0)) & ((a.y == 0.0) ^ (b.y == 0.0));
    if !(((v0.x == 0.0) ^ (v0.y == 0.0))
        & axis_aligned_orthogonal(v0, v1)
        & axis_aligned_orthogonal(v1, v2)
        & axis_aligned_orthogonal(v2, v3))
    {
        return None;
    }
    Some(sorted(pts[0], pts[2]).map(|rect| RectContour {
        rect,
        closed: true,
        clockwise: v0.x * v1.y - v0.y * v1.x > 0.0,
    }))
}

impl Path {
    /// `SkPath::isRect`: the rectangle this path's first contour traces,
    /// allowing collinear side segments, single points on a side and a
    /// missing closing side.
    pub fn is_rect(&self) -> Option<RectContour> {
        let (pts, vbs) = (&self.points[..], &self.verbs[..]);
        // getSegmentMasks() == kLine_SegmentMask.
        let mut lines = false;
        for v in vbs {
            match v {
                PathVerb::Line => lines = true,
                PathVerb::Quad | PathVerb::Conic | PathVerb::Cubic => return None,
                PathVerb::Move | PathVerb::Close => {}
            }
        }
        if !lines || pts.len() < 4 || vbs.len() < 4 {
            return None;
        }
        if let Some(rc) = trivial_rect(pts, vbs) {
            return rc;
        }

        let mut corners = 0usize;
        let mut line_start = Point::zero();
        // The first point in the rect (last of the first moves), and the
        // last point (last of the lines).
        let mut first_pt: Option<Point> = None;
        let mut last_pt: Option<Point> = None;
        let mut first_corner = Point::zero();
        let mut third_corner = Point::zero();
        let mut pi = 0usize;
        // -1 to 3; -1 is uninitialized.
        let mut directions = [-1i32; 5];
        let mut closed_or_moved = false;
        let mut auto_close = false;
        let diagonal = |a: Option<Point>, b: Option<Point>| match (a, b) {
            (Some(a), Some(b)) => {
                let d = a - b;
                d.x != 0.0 && d.y != 0.0
            }
            _ => true,
        };
        for &verb in vbs {
            match verb {
                PathVerb::Close | PathVerb::Line => {
                    let line_end = if verb == PathVerb::Close {
                        auto_close = true;
                        first_pt?
                    } else {
                        let p = pts[pi];
                        last_pt = Some(p);
                        pi += 1;
                        p
                    };
                    let delta = line_end - line_start;
                    if delta.x != 0.0 && delta.y != 0.0 {
                        return None; // diagonal
                    }
                    if !(delta.x.is_finite() && delta.y.is_finite()) {
                        return None; // infinity or NaN
                    }
                    if line_start == line_end {
                        continue; // single point on side OK
                    }
                    let next = rect_make_dir(delta.x, delta.y);
                    if corners == 0 {
                        directions[0] = next;
                        corners = 1;
                        closed_or_moved = false;
                        line_start = line_end;
                        continue;
                    }
                    if closed_or_moved {
                        return None; // closed followed by a line
                    }
                    if auto_close && next == directions[0] {
                        continue; // colinear with first
                    }
                    closed_or_moved = auto_close;
                    if directions[corners - 1] == next {
                        if corners == 3 && verb == PathVerb::Line {
                            third_corner = line_end;
                        }
                        line_start = line_end;
                        continue; // colinear segment
                    }
                    directions[corners] = next;
                    corners += 1;
                    // Opposite lines must point in opposite directions.
                    match corners {
                        2 => first_corner = line_start,
                        3 => {
                            if directions[0] ^ directions[2] != 2 {
                                return None;
                            }
                            third_corner = line_end;
                        }
                        4 => {
                            if directions[1] ^ directions[3] != 2 {
                                return None;
                            }
                        }
                        _ => return None, // too many direction changes
                    }
                    line_start = line_end;
                }
                PathVerb::Quad | PathVerb::Conic | PathVerb::Cubic => return None,
                PathVerb::Move => {
                    if corners == 0 {
                        first_pt = Some(pts[pi]);
                    } else if diagonal(first_pt, last_pt) {
                        return None;
                    }
                    line_start = pts[pi];
                    pi += 1;
                    closed_or_moved = true;
                }
            }
        }
        // Success if 4 corners and first point equals last.
        if !(3..=4).contains(&corners) {
            return None;
        }
        // Check if close generates diagonal.
        if diagonal(first_pt, last_pt) {
            return None;
        }
        Some(RectContour {
            rect: sorted(first_corner, third_corner)?,
            closed: auto_close,
            clockwise: directions[0] == ((directions[1] + 1) & 3),
        })
    }
}
