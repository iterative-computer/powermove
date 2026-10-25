//! `SkPath::isConvex` (Skia m153 `SkPath::computeConvexity` and its
//! `Convexicator`), in `f32` as Skia computes it. Graphite fills convex
//! paths with a single unstencilled wedge pass (`convexTessellatedWedges`),
//! whose coverage differs from a stencilled fill along the tessellated
//! outline (docs/engine/vector.md), so the choice must match Skia's.
//!
//! Portions are derived from Skia, Copyright Google LLC, used under the BSD
//! 3-Clause license.

use super::skpath::{P, SkPath, Verb};

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
        if cross > 0.0 { Dir::Right } else { Dir::Left }
    }

    fn add_vec(&mut self, cur: P) -> bool {
        match self.direction_change(cur) {
            d @ (Dir::Left | Dir::Right) => {
                if self.expected == Dir::Invalid {
                    self.expected = d;
                } else if d != self.expected {
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
        // The first non-zero vector after the move.
        if self.first_pt == self.last_pt && self.expected == Dir::Invalid {
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
    /// `SkPath::isConvex()`.
    pub fn is_convex(&self) -> bool {
        if !self
            .pts
            .iter()
            .all(|p| p[0].is_finite() && p[1].is_finite())
        {
            return false;
        }
        let verbs = &self.verbs;
        let mut point_count = self.pts.len();
        let leading_moves = verbs.iter().take_while(|v| **v == Verb::Move).count();
        let skip = leading_moves.saturating_sub(1);
        // The index of the last move's point.
        let mut last_move = None;
        let mut pi = 0usize;
        for v in verbs {
            if *v == Verb::Move {
                last_move = Some(pi);
            }
            pi += points_in(*v);
        }
        if let Some(lm) = last_move {
            if lm + 1 == point_count {
                // Trailing moves do not affect convexity.
                let mut k = verbs.len();
                while k > 1 && verbs[k - 1] == Verb::Move {
                    k -= 1;
                    point_count -= 1;
                }
            } else if lm != skip {
                // A move between two blocks of other verbs: more than one contour.
                return false;
            }
        }
        let start = skip.min(point_count);
        match by_sign(&self.pts[start..point_count]) {
            Some(true) => {}
            _ => return false,
        }
        let mut state = Convexicator::new();
        let mut contours = 0;
        let mut needs_close = false;
        let mut pi = 0usize;
        for v in verbs {
            let n = points_in(*v);
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
            } else if contours > 1 && *v != Verb::Move {
                return false;
            }
            pi += n;
        }
        !needs_close || state.close()
    }
}

/// Points a verb adds (`SkPathPriv::PtsInVerb`).
fn points_in(v: Verb) -> usize {
    match v {
        Verb::Move | Verb::Line => 1,
        Verb::Quad | Verb::Conic(_) => 2,
        Verb::Cubic => 3,
        Verb::Close => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn poly(pts: &[P]) -> SkPath {
        let mut p = SkPath::new();
        p.move_to(pts[0]);
        for q in &pts[1..] {
            p.line_to(*q);
        }
        p.close();
        p
    }

    #[test]
    fn convexity_follows_skia() {
        assert!(poly(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]).is_convex());
        assert!(poly(&[[0.0, 0.0], [6.0, 0.0], [6.0, 6.0]]).is_convex());
        // A dart is concave.
        assert!(!poly(&[[0.0, 0.0], [10.0, 5.0], [0.0, 10.0], [3.0, 5.0]]).is_convex());
        // A star is concave (direction changes).
        let star: Vec<P> = (0..10)
            .map(|i| {
                let a = -core::f32::consts::FRAC_PI_2 + i as f32 * core::f32::consts::PI / 5.0;
                let r = if i % 2 == 1 { 4.6 } else { 10.0 };
                [a.cos() * r, a.sin() * r]
            })
            .collect();
        assert!(!poly(&star).is_convex());
        // Two contours are not convex.
        let mut two = poly(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]);
        two.move_to([5.0, 5.0]);
        two.line_to([6.0, 5.0]);
        two.line_to([6.0, 6.0]);
        assert!(!two.is_convex());
        // A trailing move does not count.
        let mut trailing = poly(&[[0.0, 0.0], [1.0, 0.0], [1.0, 1.0]]);
        trailing.move_to([5.0, 5.0]);
        assert!(trailing.is_convex());
    }
}
