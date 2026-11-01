//! Area integration of resolved polygon edges. No point sampling or 8-bit coverage.
use i_overlay::{
    core::{fill_rule::FillRule, overlay_rule::OverlayRule},
    float::single::SingleFloatOverlay,
};
use kurbo::{BezPath, PathEl, Point, Shape};

pub(crate) fn intersect(a: &BezPath, b: &BezPath) -> BezPath {
    let a = contours(a);
    let b = contours(b);
    if a.is_empty() || b.is_empty() {
        return BezPath::new();
    }
    let shapes = a.overlay_as::<i64>(&b, OverlayRule::Intersect, FillRule::NonZero);
    let mut out = BezPath::new();
    for contour in shapes.iter().flatten() {
        if let Some(first) = contour.first() {
            out.move_to((first[0], first[1]));
            for p in &contour[1..] {
                out.line_to((p[0], p[1]));
            }
            out.close_path();
        }
    }
    out
}
fn contours(p: &BezPath) -> Vec<Vec<[f64; 2]>> {
    let mut out = Vec::new();
    let mut c = Vec::new();
    for el in p.iter() {
        match el {
            PathEl::MoveTo(p) => {
                if c.len() > 2 {
                    out.push(std::mem::take(&mut c));
                } else {
                    c.clear();
                }
                c.push([p.x, p.y]);
            }
            PathEl::LineTo(p) => c.push([p.x, p.y]),
            PathEl::ClosePath => {}
            _ => unreachable!("coverage consumes flattened outlines"),
        }
    }
    if c.len() > 2 {
        out.push(c);
    }
    out
}
#[derive(Debug, Default)]
pub(crate) struct Mask {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub values: Vec<f32>,
}
impl Mask {
    pub fn get(&self, x: u32, y: u32) -> f32 {
        if x < self.x || y < self.y || x >= self.x + self.w || y >= self.y + self.h {
            return 0.;
        }
        self.values[((y - self.y) * self.w + x - self.x) as usize]
    }
    pub fn new(path: &BezPath, w: u32, h: u32) -> Self {
        if path.elements().is_empty() {
            return Self::default();
        }
        let b = path.bounding_box();
        let (x0, y0, x1, y1) = (
            b.x0.floor().clamp(0., w as f64) as u32,
            b.y0.floor().clamp(0., h as f64) as u32,
            b.x1.ceil().clamp(0., w as f64) as u32,
            b.y1.ceil().clamp(0., h as f64) as u32,
        );
        if x1 <= x0 || y1 <= y0 {
            return Self::default();
        }
        let (mw, mh) = (x1 - x0, y1 - y0);
        let mut cells = vec![0.0f64; ((mw + 1) * mh) as usize];
        let mut edge = |a: Point, b: Point| {
            if a.y == b.y {
                return;
            }
            let (lo, hi, sign) = if a.y < b.y { (a, b, 1.) } else { (b, a, -1.) };
            let slope = (hi.x - lo.x) / (hi.y - lo.y);
            let row0 = (lo.y.floor().max(y0 as f64) as u32).min(y1);
            let row1 = (hi.y.ceil().max(y0 as f64) as u32).min(y1);
            for y in row0..row1 {
                let top = lo.y.max(y as f64);
                let bot = hi.y.min((y + 1) as f64);
                let dy = bot - top;
                let xa = lo.x + (top - lo.y) * slope;
                let xb = lo.x + (bot - lo.y) * slope;
                let (min, max) = (xa.min(xb), xa.max(xb));
                let first = min.floor().clamp(x0 as f64, x1 as f64) as u32;
                let last = max.ceil().clamp(x0 as f64, x1 as f64) as u32;
                let row = ((y - y0) * (mw + 1)) as usize;
                // Store a difference row: after the last crossed cell the edge's
                // contribution is the entire signed height, irrespective of x.
                let mut previous = 0.;
                for x in first..last {
                    let area = if (xb - xa).abs() < 1e-10 {
                        ((x + 1) as f64 - (xa + xb) * 0.5).clamp(0., 1.) * dy
                    } else {
                        // Integrate clamp(x+1-edge(y),0,1) exactly. Split at
                        // the two cell boundaries to avoid catastrophic cancellation.
                        let mut ts = [
                            0.,
                            1.,
                            (((x + 1) as f64 - xa) / (xb - xa)).clamp(0., 1.),
                            ((x as f64 - xa) / (xb - xa)).clamp(0., 1.),
                        ];
                        ts.sort_by(f64::total_cmp);
                        ts.windows(2)
                            .map(|t| {
                                let f =
                                    |u: f64| ((x + 1) as f64 - (xa + (xb - xa) * u)).clamp(0., 1.);
                                (f(t[0]) + f(t[1])) * 0.5 * (t[1] - t[0]) * dy
                            })
                            .sum()
                    };
                    cells[row + (x - x0) as usize] += (area - previous) * sign;
                    previous = area;
                }
                cells[row + (last - x0) as usize] += (dy - previous) * sign;
            }
        };
        let mut first = None;
        let mut current = None;
        for el in path.iter() {
            match el {
                PathEl::MoveTo(p) => {
                    if let (Some(a), Some(b)) = (current, first) {
                        edge(a, b);
                    }
                    first = Some(p);
                    current = Some(p);
                }
                PathEl::LineTo(p) => {
                    if let Some(a) = current {
                        edge(a, p);
                    }
                    current = Some(p);
                }
                PathEl::ClosePath => {
                    if let (Some(a), Some(b)) = (current, first) {
                        edge(a, b);
                    }
                    current = first;
                }
                _ => unreachable!(),
            }
        }
        if let (Some(a), Some(b)) = (current, first) {
            edge(a, b);
        }
        let mut values = Vec::with_capacity((mw * mh) as usize);
        for row in cells.chunks_exact((mw + 1) as usize) {
            let mut sum = 0.;
            for v in &row[..mw as usize] {
                sum += v;
                values.push(sum.abs().clamp(0., 1.) as f32);
            }
        }
        Self {
            x: x0,
            y: y0,
            w: mw,
            h: mh,
            values,
        }
    }
}
