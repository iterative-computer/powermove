//! Four-surface text/artwork extrusion. Studio's authored pixel coordinates are
//! y-down; the mesh is y-up with its front at z=0 and its back at -depth.
use crate::{Geometry, Group, earcut::earcut};
pub type Point = [f64; 2];
#[derive(Clone, Debug)]
pub struct Outline {
    pub outer: Vec<Point>,
    pub holes: Vec<Vec<Point>>,
    pub color: [f32; 4],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bevel {
    None,
    Angular,
    Concave,
    Convex,
}
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub depth: f64,
    pub bevel: Bevel,
    pub bevel_depth: f64,
    pub hole_bevel_depth: f64,
    pub steps: usize,
}
#[derive(Clone, Debug)]
pub struct Extrusion {
    pub geometry: Geometry,
    pub colors: Vec<[f32; 4]>,
}
fn area(r: &[Point]) -> f64 {
    r.iter()
        .zip(r.iter().cycle().skip(1))
        .take(r.len())
        .map(|(a, b)| a[0] * b[1] - b[0] * a[1])
        .sum::<f64>()
        * 0.5
}
fn unit(v: Point) -> Point {
    let n = v[0].hypot(v[1]);
    if n > 0. {
        [v[0] / n, v[1] / n]
    } else {
        [0., 0.]
    }
}
fn edge(a: Point, b: Point) -> Point {
    unit([b[0] - a[0], b[1] - a[1]])
}
fn normal(v: [f64; 3]) -> [f32; 3] {
    let n = v.iter().map(|x| x * x).sum::<f64>().sqrt();
    v.map(|x| if n > 0. { (x / n) as f32 } else { 0. })
}
#[derive(Default)]
struct Surface {
    vertices: std::rc::Rc<std::cell::Cell<usize>>,
    p: Vec<f32>,
    n: Vec<f32>,
    uv: Vec<f32>,
    colors: Vec<[f32; 4]>,
}
impl Surface {
    fn vertex(
        &mut self,
        p: [f64; 3],
        n: [f32; 3],
        uv: Point,
        color: [f32; 4],
    ) -> Result<(), String> {
        if p.iter()
            .chain(uv.iter())
            .any(|v| !v.is_finite() || v.abs() > 1e9)
            || n.iter().chain(color.iter()).any(|v| !v.is_finite())
        {
            return Err("Extrusion contains invalid vertex data".into());
        }
        if self.vertices.get() >= 500_000 {
            return Err("Extrusion exceeds the 500,000 vertex budget".into());
        }
        self.vertices.set(self.vertices.get() + 1);
        self.p.extend(p.map(|x| x as f32));
        self.n.extend(n);
        self.uv.extend(uv.map(|x| x as f32));
        self.colors.push(color);
        Ok(())
    }
}
pub fn build(outlines: &[Outline], o: Options) -> Result<Extrusion, String> {
    if !o.depth.is_finite()
        || !(0. ..=10000.).contains(&o.depth)
        || !o.bevel_depth.is_finite()
        || !(0. ..=1000.).contains(&o.bevel_depth)
        || !o.hole_bevel_depth.is_finite()
        || !(0. ..=100.).contains(&o.hole_bevel_depth)
        || !(1..=12).contains(&o.steps)
    {
        return Err("Invalid extrusion options".into());
    }
    let points = outlines
        .iter()
        .flat_map(|o| std::iter::once(&o.outer).chain(o.holes.iter()))
        .flatten();
    let mut bounds = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let mut count = 0;
    for p in points {
        count += 1;
        if count > 100_000 || p.iter().any(|x| !x.is_finite() || x.abs() > 1e9) {
            return Err("Extrusion outline exceeds its coordinate/point budget".into());
        }
        bounds[0] = bounds[0].min(p[0]);
        bounds[1] = bounds[1].min(p[1]);
        bounds[2] = bounds[2].max(p[0]);
        bounds[3] = bounds[3].max(p[1]);
    }
    let (w, h) = (
        (bounds[2] - bounds[0]).max(1e-5),
        (bounds[3] - bounds[1]).max(1e-5),
    );
    let vertices = std::rc::Rc::new(std::cell::Cell::new(0));
    let mut surfaces: [Surface; 4] = std::array::from_fn(|_| Surface {
        vertices: vertices.clone(),
        ..Default::default()
    });
    let b = if o.bevel == Bevel::None {
        0.
    } else {
        o.bevel_depth.min(o.depth / 2.)
    };
    let steps = if o.bevel == Bevel::Angular {
        1
    } else {
        o.steps
    };
    for outline in outlines {
        let convert = |r: &[Point], ccw: bool| {
            let mut ring: Vec<Point> = r.iter().map(|p| [p[0], -p[1]]).collect();
            let original = ring.clone();
            ring = original
                .iter()
                .enumerate()
                .filter(|(i, p)| {
                    let prev = original[(*i + original.len() - 1) % original.len()];
                    (p[0] - prev[0]).hypot(p[1] - prev[1]) > 1e-5
                })
                .map(|(_, p)| *p)
                .collect();
            if (area(&ring) > 0.) != ccw {
                ring.reverse();
            }
            ring
        };
        let outer = convert(&outline.outer, true);
        if outer.len() < 3 {
            continue;
        }
        let rings: Vec<_> = std::iter::once(outer)
            .chain(
                outline
                    .holes
                    .iter()
                    .map(|h| convert(h, false))
                    .filter(|h| h.len() >= 3),
            )
            .collect();
        let mut flat = vec![];
        let mut holes = vec![];
        for (i, r) in rings.iter().enumerate() {
            if i > 0 {
                holes.push(flat.len() / 2);
            }
            flat.extend(r.iter().flatten());
        }
        for tri in earcut(&flat, &holes).chunks_exact(3) {
            for &i in tri {
                let (x, y) = (flat[i * 2], flat[i * 2 + 1]);
                surfaces[0].vertex(
                    [x, y, 0.],
                    [0., 0., 1.],
                    [(x - bounds[0]) / w, (-y - bounds[1]) / h],
                    outline.color,
                )?;
            }
            if o.depth > 0. {
                for &i in tri.iter().rev() {
                    let (x, y) = (flat[i * 2], flat[i * 2 + 1]);
                    surfaces[3].vertex(
                        [x, y, -o.depth],
                        [0., 0., -1.],
                        [(x - bounds[0]) / w, (-y - bounds[1]) / h],
                        outline.color,
                    )?;
                }
            }
        }
        if o.depth == 0. {
            continue;
        }
        for (ri, ring) in rings.iter().enumerate() {
            let count = ring.len();
            let mut distances = vec![0.];
            for i in 0..count {
                let (a, q) = (ring[i], ring[(i + 1) % count]);
                distances.push(distances[i] + (q[0] - a[0]).hypot(q[1] - a[1]));
            }
            let perimeter = distances[count];
            let limit = area(ring).abs() / perimeter.max(1.) * 0.4;
            let bevel = (b * if ri > 0 {
                o.hole_bevel_depth / 100.
            } else {
                1.
            })
            .min(limit);
            let offsets: Vec<_> = (0..count)
                .map(|i| {
                    let p = ring[i];
                    let (d1, d2) = (
                        edge(ring[(i + count - 1) % count], p),
                        edge(p, ring[(i + 1) % count]),
                    );
                    let (n1, n2) = ([d1[1], -d1[0]], [d2[1], -d2[0]]);
                    let m = unit([n1[0] + n2[0], n1[1] + n2[1]]);
                    let f = (1. / (m[0] * n1[0] + m[1] * n1[1]).max(0.25)).min(2.);
                    [m[0] * f, m[1] * f]
                })
                .collect();
            let mut profile: Vec<(f64, f64, usize)> = vec![];
            if bevel > 0. {
                for j in 0..=steps {
                    let t = j as f64 / steps as f64;
                    let a = t * std::f64::consts::FRAC_PI_2;
                    let (r, z) = match o.bevel {
                        Bevel::Convex => (a.sin(), 1. - a.cos()),
                        Bevel::Concave => (1. - a.cos(), a.sin()),
                        _ => (t, t),
                    };
                    profile.push((r * bevel, -z * bevel, 1));
                }
            } else {
                profile.push((0., 0., 2));
            }
            profile.push((bevel, -o.depth + bevel, 2));
            if bevel > 0. {
                for j in (0..steps).rev() {
                    let (r, z, _) = profile[j];
                    profile.push((r, -o.depth - z, 1));
                }
            }
            let mut lengths = vec![0.];
            for j in 1..profile.len() {
                lengths.push(
                    lengths[j - 1]
                        + (profile[j].0 - profile[j - 1].0).hypot(profile[j].1 - profile[j - 1].1),
                );
            }
            for j in 0..profile.len() - 1 {
                let (a, next) = (profile[j], profile[j + 1]);
                if (next.0 - a.0).hypot(next.1 - a.1) < 1e-8 {
                    continue;
                }
                for i in 0..count {
                    let k = (i + 1) % count;
                    for (index, row, u) in [
                        (i, j, distances[i]),
                        (i, j + 1, distances[i]),
                        (k, j + 1, distances[i + 1]),
                        (i, j, distances[i]),
                        (k, j + 1, distances[i + 1]),
                        (k, j, distances[i + 1]),
                    ] {
                        let p = ring[index];
                        let offset = offsets[index];
                        let r = profile[row];
                        let (prev_row, next_row) =
                            (row.saturating_sub(1), (row + 1).min(profile.len() - 1));
                        let slope = if o.bevel == Bevel::Angular {
                            (next.0 - a.0) / (a.1 - next.1).max(1e-8)
                        } else {
                            (profile[next_row].0 - profile[prev_row].0)
                                / (-(profile[next_row].1 - profile[prev_row].1)).max(1e-8)
                        };
                        let e = edge(ring[i], ring[k]);
                        let (e1, e2) = (
                            edge(ring[(index + count - 1) % count], p),
                            edge(p, ring[(index + 1) % count]),
                        );
                        let n = if e1[0] * e2[0] + e1[1] * e2[1] > 0.7 {
                            unit([e1[1] + e2[1], -e1[0] - e2[0]])
                        } else {
                            [e[1], -e[0]]
                        };
                        surfaces[next.2].vertex(
                            [p[0] + offset[0] * r.0, p[1] + offset[1] * r.0, r.1],
                            normal([n[0], n[1], slope]),
                            [
                                u / perimeter.max(1.),
                                lengths[row] / lengths.last().copied().unwrap_or(1.).max(1.),
                            ],
                            outline.color,
                        )?;
                    }
                }
            }
        }
    }
    let mut geometry = Geometry {
        positions: vec![],
        normals: vec![],
        uvs: vec![],
        indices: vec![],
        indexed: false,
        groups: vec![],
    };
    let mut colors = vec![];
    for (index, surface) in surfaces.into_iter().enumerate() {
        geometry.groups.push(Group {
            start: (geometry.positions.len() / 3) as u32,
            count: (surface.p.len() / 3) as u32,
            material_index: index as u32,
        });
        geometry.positions.extend(surface.p);
        geometry.normals.extend(surface.n);
        geometry.uvs.extend(surface.uv);
        colors.extend(surface.colors);
    }
    if geometry.positions.len() / 3 > 500_000 {
        return Err("Extrusion exceeds the 500,000 vertex budget".into());
    }
    geometry.indices = (0..(geometry.positions.len() / 3) as u32).collect();
    Ok(Extrusion { geometry, colors })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn square() -> Outline {
        Outline {
            outer: vec![[0., 0.], [10., 0.], [10., 10.], [0., 10.]],
            holes: vec![vec![[3., 3.], [3., 7.], [7., 7.], [7., 3.]]],
            color: [1.; 4],
        }
    }
    #[test]
    fn caps_keep_glyph_counters_and_all_four_surfaces() {
        for bevel in [Bevel::None, Bevel::Angular, Bevel::Concave, Bevel::Convex] {
            let e = build(
                &[square()],
                Options {
                    depth: 5.,
                    bevel,
                    bevel_depth: 1.,
                    hole_bevel_depth: 50.,
                    steps: 6,
                },
            )
            .unwrap();
            let g = &e.geometry;
            let cap = g.groups[0];
            let mut area = 0.;
            for p in g.positions[cap.start as usize * 3..(cap.start + cap.count) as usize * 3]
                .chunks_exact(9)
            {
                area += ((p[3] - p[0]) * (p[7] - p[1]) - (p[4] - p[1]) * (p[6] - p[0])).abs() * 0.5;
            }
            assert!((area - 84.).abs() < 1e-5);
            assert!(g.groups[2].count > 0);
            assert!(g.groups[3].count > 0);
            assert_eq!(g.groups[1].count > 0, bevel != Bevel::None);
            assert_eq!(e.colors.len(), g.positions.len() / 3);
            assert!(
                g.normals
                    .chunks_exact(3)
                    .all(|n| (n.iter().map(|v| v * v).sum::<f32>() - 1.).abs() < 1e-5)
            );
            assert_eq!(
                g.positions
                    .chunks_exact(3)
                    .map(|p| p[2])
                    .fold(f32::INFINITY, f32::min),
                -5.
            );
        }
    }
}
