//! Skia path verb storage and rectangle detection used by the legacy text raster.
//! Portions derived from Skia under the BSD 3-Clause license.

pub type P = [f32; 2];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verb {
    Move,
    Line,
    Quad,
    Conic(f32),
    Cubic,
    Close,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkPath {
    pub verbs: Vec<Verb>,
    pub pts: Vec<P>,
    last_move: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat {
    pub sx: f32,
    pub kx: f32,
    pub tx: f32,
    pub ky: f32,
    pub sy: f32,
    pub ty: f32,
}

impl SkPath {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn move_to(&mut self, p: P) {
        if let Some(Verb::Move) = self.verbs.last() {
            *self.pts.last_mut().unwrap() = p;
        } else {
            self.last_move = Some(self.pts.len());
            self.pts.push(p);
            self.verbs.push(Verb::Move);
        }
    }

    fn ensure_move(&mut self) {
        match self.verbs.last() {
            None => self.move_to([0.0, 0.0]),
            Some(Verb::Close) => {
                let p = self.last_move.map_or([0.0, 0.0], |i| self.pts[i]);
                self.move_to(p);
            }
            _ => {}
        }
    }

    pub fn line_to(&mut self, p: P) {
        self.ensure_move();
        self.pts.push(p);
        self.verbs.push(Verb::Line);
    }

    pub fn quad_to(&mut self, p1: P, p2: P) {
        self.ensure_move();
        self.pts.extend([p1, p2]);
        self.verbs.push(Verb::Quad);
    }

    pub fn conic_to(&mut self, p1: P, p2: P, w: f32) {
        self.ensure_move();
        if w <= 0.0 {
            self.line_to(p2);
            return;
        }
        self.pts.extend([p1, p2]);
        if w == 1.0 {
            self.verbs.push(Verb::Quad);
        } else if w.is_finite() {
            self.verbs.push(Verb::Conic(w));
        } else {
            self.verbs.push(Verb::Line);
            self.verbs.push(Verb::Line);
        }
    }

    pub fn cubic_to(&mut self, p1: P, p2: P, p3: P) {
        self.ensure_move();
        self.pts.extend([p1, p2, p3]);
        self.verbs.push(Verb::Cubic);
    }

    pub fn close(&mut self) {
        if !self.verbs.is_empty() && self.verbs.last() != Some(&Verb::Close) {
            self.ensure_move();
            self.verbs.push(Verb::Close);
        }
    }

    pub fn segments(&self) -> Vec<(Verb, Vec<P>)> {
        let mut out = Vec::new();
        let mut pi = 0usize;
        let mut last = [0.0f32; 2];
        let mut start = [0.0f32; 2];
        for v in &self.verbs {
            match v {
                Verb::Move => {
                    last = self.pts[pi];
                    start = last;
                    pi += 1;
                    out.push((*v, vec![last]));
                }
                Verb::Line => {
                    let p = self.pts[pi];
                    out.push((*v, vec![last, p]));
                    last = p;
                    pi += 1;
                }
                Verb::Quad | Verb::Conic(_) => {
                    let (a, b) = (self.pts[pi], self.pts[pi + 1]);
                    out.push((*v, vec![last, a, b]));
                    last = b;
                    pi += 2;
                }
                Verb::Cubic => {
                    let (a, b, c) = (self.pts[pi], self.pts[pi + 1], self.pts[pi + 2]);
                    out.push((*v, vec![last, a, b, c]));
                    last = c;
                    pi += 3;
                }
                Verb::Close => {
                    out.push((*v, vec![last, start]));
                    last = start;
                }
            }
        }
        out
    }

    pub fn is_line(&self) -> Option<[P; 2]> {
        (self.verbs.len() == 2 && self.verbs[0] == Verb::Move && self.verbs[1] == Verb::Line)
            .then(|| [self.pts[0], self.pts[1]])
    }

    pub fn is_rect(&self) -> Option<([f32; 4], bool)> {
        is_rect_contour(&self.pts, &self.verbs)
    }
}

fn rect_make_dir(dx: f32, dy: f32) -> i32 {
    i32::from(dx != 0.0) | (i32::from(dx > 0.0 || dy > 0.0) << 1)
}

fn is_rect_contour(pts: &[P], verbs: &[Verb]) -> Option<([f32; 4], bool)> {
    if verbs
        .iter()
        .any(|v| matches!(v, Verb::Quad | Verb::Conic(_) | Verb::Cubic))
        || !verbs.contains(&Verb::Line)
        || pts.len() < 4
        || verbs.len() < 4
    {
        return None;
    }
    // trivial_rect: move, line, line, line, close with four points.
    if pts.len() == 4 && verbs == [Verb::Move, Verb::Line, Verb::Line, Verb::Line, Verb::Close] {
        let v = |a: usize, b: usize| [pts[b][0] - pts[a][0], pts[b][1] - pts[a][1]];
        let (v0, v1, v2, v3) = (v(0, 1), v(1, 2), v(2, 3), v(3, 0));
        let ortho = |a: P, b: P| ((a[0] == 0.0) ^ (b[0] == 0.0)) & ((a[1] == 0.0) ^ (b[1] == 0.0));
        if ((v0[0] == 0.0) ^ (v0[1] == 0.0)) & ortho(v0, v1) & ortho(v1, v2) & ortho(v2, v3) {
            let r = sorted([pts[0][0], pts[0][1], pts[2][0], pts[2][1]]);
            return Some((r, true));
        }
        return None;
    }
    let mut corners = 0;
    let mut line_start = [0.0f32, 0.0];
    let mut first_pt: Option<usize> = None;
    let mut last_pt: Option<usize> = None;
    let mut first_corner = [0.0f32; 2];
    let mut third_corner = [0.0f32; 2];
    let mut directions = [-1i32; 5];
    let mut closed_or_moved = false;
    let mut auto_close = false;
    let mut pi = 0usize;
    for &verb in verbs {
        match verb {
            Verb::Close | Verb::Line => {
                if verb == Verb::Close {
                    auto_close = true;
                } else {
                    last_pt = Some(pi);
                }
                let line_end = if verb == Verb::Close {
                    pts[first_pt?]
                } else {
                    let p = pts[pi];
                    pi += 1;
                    p
                };
                let d = [line_end[0] - line_start[0], line_end[1] - line_start[1]];
                if d[0] != 0.0 && d[1] != 0.0 {
                    return None;
                }
                if !(d[0].is_finite() && d[1].is_finite()) {
                    return None;
                }
                if line_start == line_end {
                    continue;
                }
                let next = rect_make_dir(d[0], d[1]);
                if corners == 0 {
                    directions[0] = next;
                    corners = 1;
                    closed_or_moved = false;
                    line_start = line_end;
                    continue;
                }
                if closed_or_moved {
                    return None;
                }
                if auto_close && next == directions[0] {
                    continue;
                }
                closed_or_moved = auto_close;
                if directions[corners - 1] == next {
                    if corners == 3 && verb == Verb::Line {
                        third_corner = line_end;
                    }
                    line_start = line_end;
                    continue;
                }
                directions[corners] = next;
                corners += 1;
                match corners {
                    2 => first_corner = line_start,
                    3 => {
                        if (directions[0] ^ directions[2]) != 2 {
                            return None;
                        }
                        third_corner = line_end;
                    }
                    4 => {
                        if (directions[1] ^ directions[3]) != 2 {
                            return None;
                        }
                    }
                    _ => return None,
                }
                line_start = line_end;
            }
            Verb::Move => {
                if corners == 0 {
                    first_pt = Some(pi);
                } else {
                    let (f, l) = (pts[first_pt?], pts[last_pt?]);
                    if f[0] - l[0] != 0.0 && f[1] - l[1] != 0.0 {
                        return None;
                    }
                }
                line_start = pts[pi];
                pi += 1;
                closed_or_moved = true;
            }
            _ => return None,
        }
    }
    if !(3..=4).contains(&corners) {
        return None;
    }
    let (f, l) = (pts[first_pt?], pts[last_pt.unwrap_or(first_pt?)]);
    if f[0] - l[0] != 0.0 && f[1] - l[1] != 0.0 {
        return None;
    }
    let r = sorted([
        first_corner[0],
        first_corner[1],
        third_corner[0],
        third_corner[1],
    ]);
    Some((r, auto_close))
}

fn sorted(r: [f32; 4]) -> [f32; 4] {
    [
        r[0].min(r[2]),
        r[1].min(r[3]),
        r[0].max(r[2]),
        r[1].max(r[3]),
    ]
}
