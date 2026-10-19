//! Skia chrome/m152 (747cf654) `SkPathPriv::ComputeConvexity` and its
//! `Convexicator` (src/core/SkPathPriv.cpp:422-648), in f32. The analytic scan converter selects a different edge walk for
//! convex paths, so it must classify their control polygons the same way.
//!
//! Portions are derived from Skia, Copyright Google LLC, used under the BSD
//! 3-Clause license.

use crate::{Path, PathSegment};
type P = [f32; 2];
#[derive(Clone, Copy, PartialEq)]
enum Verb {
    Move,
    Line,
    Quad,
    Conic,
    Cubic,
    Close,
}
struct SkPath {
    pts: alloc::vec::Vec<P>,
    verbs: alloc::vec::Vec<Verb>,
}

impl Path {
    /// Skia's control-polygon convexity classification. A convex path's
    /// analytic scan converter has a different edge walk from a winding fill.
    pub fn is_convex(&self) -> bool {
        let mut p = SkPath {
            pts: alloc::vec::Vec::new(),
            verbs: alloc::vec::Vec::new(),
        };
        let xy = |v: crate::Point| [v.x, v.y];
        for s in self.segments() {
            match s {
                PathSegment::MoveTo(v) => {
                    p.verbs.push(Verb::Move);
                    p.pts.push(xy(v));
                }
                PathSegment::LineTo(v) => {
                    p.verbs.push(Verb::Line);
                    p.pts.push(xy(v));
                }
                PathSegment::QuadTo(c, v) => {
                    p.verbs.push(Verb::Quad);
                    p.pts.extend([xy(c), xy(v)]);
                }
                PathSegment::ConicTo(c, v, _) => {
                    p.verbs.push(Verb::Conic);
                    p.pts.extend([xy(c), xy(v)]);
                }
                PathSegment::CubicTo(a, b, v) => {
                    p.verbs.push(Verb::Cubic);
                    p.pts.extend([xy(a), xy(b), xy(v)]);
                }
                PathSegment::Close => p.verbs.push(Verb::Close),
            }
        }
        p.is_convex()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Dir {
    Invalid,
    Left,
    Right,
    Straight,
    Backwards,
    Unknown,
}

fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}

struct Convexicator {
    first_pt: P,
    last_pt: P,
    first_vec: P,
    last_vec: P,
    expected: Dir,
    /// `fFirstDirection` is kUnknown (neither CW nor CCW).
    first_direction_unknown: bool,
    reversals: u32,
}

impl Convexicator {
    fn new() -> Self {
        Self {
            first_pt: [0.0; 2],
            last_pt: [0.0; 2],
            first_vec: [0.0; 2],
            last_vec: [0.0; 2],
            expected: Dir::Invalid,
            first_direction_unknown: true,
            reversals: 0,
        }
    }

    fn set_move_pt(&mut self, p: P) {
        self.first_pt = p;
        self.last_pt = p;
        self.expected = Dir::Invalid;
    }

    fn direction_change(&self, cur: P) -> Dir {
        let l = self.last_vec;
        let cross = l[0] * cur[1] - l[1] * cur[0];
        if !cross.is_finite() {
            return Dir::Unknown;
        }
        if cross == 0.0 {
            return if l[0] * cur[0] + l[1] * cur[1] < 0.0 {
                Dir::Backwards
            } else {
                Dir::Straight
            };
        }
        if cross > 0.0 {
            Dir::Right
        } else {
            Dir::Left
        }
    }

    fn add_vec(&mut self, cur: P) -> bool {
        match self.direction_change(cur) {
            d @ (Dir::Left | Dir::Right) => {
                if self.expected == Dir::Invalid {
                    self.expected = d;
                    self.first_direction_unknown = false;
                } else if d != self.expected {
                    self.first_direction_unknown = true;
                    return false;
                }
                self.last_vec = cur;
                true
            }
            Dir::Straight => true,
            Dir::Backwards => {
                // The path may reverse direction twice.
                self.last_vec = cur;
                self.reversals += 1;
                self.reversals < 3
            }
            Dir::Unknown | Dir::Invalid => false,
        }
    }

    fn add_pt(&mut self, p: P) -> bool {
        if self.last_pt == p {
            return true;
        }
        // The first non-zero vector after the move. The contour may have
        // doubled back to its start, so the last vector must still be zero.
        if self.first_pt == self.last_pt
            && self.expected == Dir::Invalid
            && self.last_vec == [0.0, 0.0]
        {
            self.last_vec = sub(p, self.last_pt);
            self.first_vec = self.last_vec;
        } else if !self.add_vec(sub(p, self.last_pt)) {
            return false;
        }
        self.last_pt = p;
        true
    }

    fn close(&mut self) -> bool {
        let (first_pt, first_vec) = (self.first_pt, self.first_vec);
        self.add_pt(first_pt) && self.add_vec(first_vec)
    }
}

/// `Convexicator::BySign`: more than three sign changes of either
/// coordinate's direction (cyclically) is concave.
fn by_sign(points: &[P]) -> Option<bool> {
    if points.len() <= 3 {
        return Some(true);
    }
    let sign = |v: f32| i32::from(v < 0.0);
    const NEVER: i32 = 2;
    let (mut dxes, mut dyes) = (0, 0);
    let (mut last_sx, mut last_sy) = (NEVER, NEVER);
    let mut curr = points[0];
    let mut i = 1;
    for outer in 0..2 {
        loop {
            let next = if outer == 0 {
                if i == points.len() {
                    break;
                }
                points[i]
            } else {
                points[0]
            };
            let vec = sub(next, curr);
            if vec != [0.0, 0.0] {
                if !(vec[0].is_finite() && vec[1].is_finite()) {
                    return None;
                }
                let (sx, sy) = (sign(vec[0]), sign(vec[1]));
                dxes += i32::from(sx != last_sx);
                dyes += i32::from(sy != last_sy);
                if dxes > 3 || dyes > 3 {
                    return Some(false);
                }
                last_sx = sx;
                last_sy = sy;
            }
            curr = next;
            i += 1;
            if outer == 1 {
                break;
            }
        }
    }
    Some(true)
}

impl SkPath {
    /// `SkPath::isConvex()`: `SkPath::computeConvexity` (SkPath.cpp:638-654)
    /// and `SkPathPriv::ComputeConvexity` (SkPathPriv.cpp:584-648), then
    /// `SkPathConvexity_IsConvex`.
    pub fn is_convex(&self) -> bool {
        if !self
            .pts
            .iter()
            .all(|p| p[0].is_finite() && p[1].is_finite())
        {
            return false;
        }
        // trim_trailing_moves.
        let mut verbs = &self.verbs[..];
        let mut point_count = self.pts.len();
        while let [rest @ .., Verb::Move] = verbs {
            verbs = rest;
            point_count -= 1;
        }
        if verbs.is_empty() {
            // kConvex_Degenerate.
            return true;
        }
        if by_sign(&self.pts[..point_count]) != Some(true) {
            return false;
        }
        let mut state = Convexicator::new();
        let mut contours = 0;
        let mut needs_close = false;
        let mut pi = 0usize;
        for v in verbs {
            let n = points_in(*v);
            // The last move before non-move verbs start.
            if contours == 0 {
                if *v == Verb::Move {
                    state.set_move_pt(self.pts[pi]);
                } else {
                    contours += 1;
                    needs_close = true;
                }
            }
            if contours == 1 {
                if matches!(v, Verb::Close | Verb::Move) {
                    if !state.close() {
                        return false;
                    }
                    needs_close = false;
                    contours += 1;
                } else {
                    for k in 0..n {
                        if !state.add_pt(self.pts[pi + k]) {
                            return false;
                        }
                    }
                }
            } else if *v != Verb::Move {
                // After the first contour closes, anything but spurious
                // moves is a second contour.
                return false;
            }
            pi += n;
        }
        if needs_close && !state.close() {
            return false;
        }
        // kUnknown first direction with three reversals is concave;
        // every other outcome is kConvex_CW, kConvex_CCW or
        // kConvex_Degenerate.
        !(state.first_direction_unknown && state.reversals >= 3)
    }
}

/// Points a verb adds (`SkPathPriv::PtsInVerb`).
fn points_in(v: Verb) -> usize {
    match v {
        Verb::Move | Verb::Line => 1,
        Verb::Quad | Verb::Conic => 2,
        Verb::Cubic => 3,
        Verb::Close => 0,
    }
}
