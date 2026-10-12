//! A deterministic digest of this crate's geometry over generated cases.
//!
//! Native and wasm32 builds must produce the same digest (section 26, Phase 1
//! gate: native and WASM bit-identical). `examples/digest.rs` prints it
//! natively; `examples/digest_wasm.rs` exports it from a wasm32 module that
//! `tests/wasm/digest.ts` runs under bun.

use kurbo::{Affine, BezPath, PathEl, Point};

use crate::corner::{self, CornerContent};
use crate::mask::{LayerMasks, MaskMode, MaskShape, MaskValues, layer_inverse};
use crate::path::{PathLayer, PathNode, PathValues, Vertex, path_points};
use crate::shape::{ShapeContent, ShapeKind, ShapeRaster, shape_geometry};

/// xorshift64* generator.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in [lo, hi).
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (self.next() >> 11) as f64 / (1u64 << 53) as f64 * (hi - lo)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// FNV-1a over the bits of every number.
struct Digest {
    hash: u64,
    values: u64,
}

impl Digest {
    fn f64(&mut self, value: f64) {
        for byte in value.to_bits().to_le_bytes() {
            self.hash ^= u64::from(byte);
            self.hash = self.hash.wrapping_mul(0x0000_0100_0000_01B3);
        }
        self.values += 1;
    }
    fn f32(&mut self, value: f32) {
        self.f64(f64::from(value));
    }
    fn point(&mut self, p: Point) {
        self.f64(p.x);
        self.f64(p.y);
    }
    fn affine(&mut self, a: Affine) {
        a.as_coeffs().iter().for_each(|&c| self.f64(c));
    }
    fn path(&mut self, path: &BezPath) {
        for el in path.elements() {
            match *el {
                PathEl::MoveTo(p) => {
                    self.f64(0.0);
                    self.point(p);
                }
                PathEl::LineTo(p) => {
                    self.f64(1.0);
                    self.point(p);
                }
                PathEl::QuadTo(a, b) => {
                    self.f64(2.0);
                    self.point(a);
                    self.point(b);
                }
                PathEl::CurveTo(a, b, c) => {
                    self.f64(3.0);
                    self.point(a);
                    self.point(b);
                    self.point(c);
                }
                PathEl::ClosePath => self.f64(4.0),
            }
        }
    }
}

fn vertex(rng: &mut Rng) -> Vertex {
    let tangent = rng.below(3) == 0;
    let t = |rng: &mut Rng| if tangent { rng.range(-80.0, 80.0) } else { 0.0 };
    Vertex {
        x: rng.range(-300.0, 300.0),
        y: rng.range(-300.0, 300.0),
        in_x: t(rng),
        in_y: t(rng),
        out_x: t(rng),
        out_y: t(rng),
    }
}

fn path_values(rng: &mut Rng) -> PathValues {
    let n = rng.below(6) as usize;
    PathValues {
        x: rng.range(-100.0, 100.0),
        y: rng.range(-100.0, 100.0),
        rotation: rng.range(-360.0, 360.0),
        scale_x: rng.range(-200.0, 200.0),
        scale_y: rng.range(10.0, 200.0),
        closed: rng.below(2) == 0,
        fill_enabled: rng.below(4) != 0,
        fill_opacity: rng.range(-20.0, 120.0),
        stroke_width: rng.range(-2.0, 20.0),
        stroke_opacity: rng.range(0.0, 100.0),
        trim_start: rng.range(-10.0, 100.0),
        trim_end: rng.range(0.0, 110.0),
        trim_offset: rng.range(-720.0, 720.0),
        copies: rng.range(0.0, 8.0),
        repeat_x: rng.range(-50.0, 50.0),
        repeat_y: rng.range(-50.0, 50.0),
        repeat_rotation: rng.range(-90.0, 90.0),
        vertices: (0..n).map(|_| vertex(rng)).collect(),
    }
}

/// Digest and number of values hashed over `cases` generated cases per family.
pub fn digest(cases: u32) -> (u64, u64) {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut d = Digest {
        hash: 0xCBF2_9CE4_8422_2325,
        values: 0,
    };
    let kinds = [
        ShapeKind::Rect,
        ShapeKind::Ellipse,
        ShapeKind::Polygon,
        ShapeKind::Star,
        ShapeKind::Line,
    ];
    for _ in 0..cases {
        let independent = rng.below(2) == 0;
        let mut radius = || {
            if rng.below(5) == 0 {
                None
            } else {
                Some(rng.range(-10.0, 300.0))
            }
        };
        let corner_radii = [radius(), radius(), radius(), radius()];
        let shape = ShapeContent {
            shape: kinds[rng.below(5) as usize],
            w: rng.range(0.0, 1200.0),
            h: rng.range(0.0, 1200.0),
            radius: rng.range(0.0, 400.0),
            independent_corners: independent,
            corner_radii,
            smoothing: if rng.below(2) == 0 {
                0.0
            } else {
                rng.range(0.0, 120.0)
            },
            stroke: rng.range(-2.0, 30.0),
            points: rng.range(0.0, 14.0),
        };
        let corners: CornerContent = shape.corners();
        corner::corner_radii(&corners)
            .iter()
            .for_each(|&r| d.f64(r));
        d.f64(corner::corner_inset(&corners));
        d.path(&corner::rounded_rect_path(&corners));
        if let Ok(g) = shape_geometry(&shape) {
            d.path(&g.path);
            match g.stroke {
                Some(s) => {
                    d.f64(s.style.width);
                    d.f64(s.style.start_cap as u8 as f64);
                    d.f64(s.style.join as u8 as f64);
                }
                None => d.f64(-1.0),
            }
        }
        let raster = ShapeRaster::new(&shape, rng.range(0.1, 40.0));
        d.affine(raster.shape_to_layer());
        d.f64(raster.width);

        let nodes: Vec<PathNode> = (0..1 + rng.below(3))
            .map(|i| PathNode {
                id: format!("p{i}"),
                parent: (rng.below(2) == 0).then(|| format!("p{}", rng.below(3))),
                values: path_values(&mut rng),
            })
            .collect();
        for node in &nodes {
            path_points(&node.values)
                .into_iter()
                .for_each(|p| d.point(p));
        }
        let layer = PathLayer::new(&nodes);
        layer.traced.iter().for_each(|p| d.path(p));
        let raster = layer.raster(rng.range(0.1, 16.0));
        let base = layer.bitmap_transform(&raster);
        d.f64(raster.width);
        d.f64(raster.height);
        for draw in &layer.draws {
            d.affine(draw.transform);
            d.affine(draw.transform_from(base).affine());
            d.f64(draw.fill.unwrap_or(-1.0));
            d.f64(draw.stroke.as_ref().map_or(-1.0, |s| s.style.width));
        }
        let b = layer.bounds;
        [b.x0, b.y0, b.x1, b.y1].into_iter().for_each(|v| d.f64(v));

        let masks: Vec<MaskValues> = (0..rng.below(10))
            .map(|_| MaskValues {
                x: rng.range(-200.0, 200.0),
                y: rng.range(-200.0, 200.0),
                w: rng.range(-300.0, 300.0),
                h: rng.range(-300.0, 300.0),
                rotation: rng.range(-180.0, 180.0),
                feather: rng.range(-5.0, 40.0),
                shape: if rng.below(2) == 0 {
                    MaskShape::Rect
                } else {
                    MaskShape::Ellipse
                },
                mode: if rng.below(3) == 0 {
                    MaskMode::Subtract
                } else {
                    MaskMode::Add
                },
                path: (rng.below(8) == 0).then(|| path_values(&mut rng)),
            })
            .collect();
        match LayerMasks::new(&masks) {
            LayerMasks::None => d.f64(0.0),
            LayerMasks::Parametric(p) => {
                let world = [
                    rng.range(-3.0, 3.0),
                    rng.range(-3.0, 3.0),
                    rng.range(-3.0, 3.0),
                    rng.range(-3.0, 3.0),
                    rng.range(-500.0, 500.0),
                    rng.range(-500.0, 500.0),
                ];
                d.affine(layer_inverse(world).unwrap_or(Affine::IDENTITY));
                p.masks.iter().for_each(|m| d.path(&m.outline()));
                for _ in 0..16 {
                    let at = Point::new(rng.range(-300.0, 300.0), rng.range(-300.0, 300.0));
                    d.f32(p.coverage(at));
                }
            }
            LayerMasks::Path(p) => {
                let world = [
                    rng.range(-3.0, 3.0),
                    rng.range(-3.0, 3.0),
                    rng.range(-3.0, 3.0),
                    rng.range(-3.0, 3.0),
                    rng.range(-500.0, 500.0),
                    rng.range(-500.0, 500.0),
                ];
                for m in &p.masks {
                    d.path(&m.path);
                    d.affine(m.transform);
                    d.affine(m.transform_from(world).affine());
                    d.f64(m.blur_sigma(Affine::new(world)));
                }
            }
        }
    }
    (d.hash, d.values)
}
