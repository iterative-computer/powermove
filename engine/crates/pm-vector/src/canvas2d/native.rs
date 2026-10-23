//! Local-space patch packets for the native Graphite render steps.
use super::{
    skpath::{Mat, P, SkPath, Verb},
    stroke::{self, Cap, StrokeParams},
    tess::{self, Patch, Vx},
};
use pm_text::render::graphite::Geometry;

pub(super) fn fill(path: &SkPath, m: &Mat, winding: bool, even_odd: bool) -> Geometry {
    let vx = Vx::of(m);
    let mut output = Vec::new();
    let mut max_n4 = 1.0f32;
    let mut patches = Vec::new();
    let mut first = None;
    let mut last = [0.; 2];
    let mut sum = [0.; 2];
    let mut count = 0u32;
    let flush = |first: Option<P>,
                 last: P,
                 mut sum: P,
                 mut count: u32,
                 patches: &mut Vec<Patch>,
                 output: &mut Vec<[[f32; 4]; 5]>| {
        let Some(first) = first else {
            return;
        };
        if last != first {
            tess::curve_patches(Verb::Line, &[last, first], &vx, patches);
            sum[0] += first[0];
            sum[1] += first[1];
            count += 1;
        }
        let k = 1. / count.max(1) as f32;
        let fan = [sum[0] * k, sum[1] * k];
        for patch in patches.drain(..) {
            let p = patch.p;
            let tail = patch.w.map_or([p[2][0], p[2][1], p[3][0], p[3][1]], |w| {
                [p[2][0], p[2][1], w, 0.]
            });
            output.push([
                [p[0][0], p[0][1], p[1][0], p[1][1]],
                tail,
                [m.sx, m.ky, m.kx, m.sy],
                [m.tx, m.ty, 0., f32::from(patch.w.is_some())],
                [fan[0], fan[1], 0., 0.],
            ]);
        }
    };
    for (verb, pts) in path.segments() {
        match verb {
            Verb::Move => {
                flush(first, last, sum, count, &mut patches, &mut output);
                first = Some(pts[0]);
                last = pts[0];
                sum = [0.; 2];
                count = 0;
            }
            Verb::Close => {
                flush(first, last, sum, count, &mut patches, &mut output);
                first = None;
            }
            _ => {
                let n4 = match verb {
                    Verb::Cubic => tess::cubic_p4(&[pts[0], pts[1], pts[2], pts[3]], &vx),
                    Verb::Conic(w) => {
                        let n2 = tess::conic_p2(&[pts[0], pts[1], pts[2]], w, &vx);
                        n2 * n2
                    }
                    Verb::Quad => {
                        let q = [
                            -2. * pts[1][0] + pts[0][0] + pts[2][0],
                            -2. * pts[1][1] + pts[0][1] + pts[2][1],
                        ];
                        let (c0, c1) = vx.columns();
                        let v = [c0[0] * q[0] + c1[0] * q[1], c0[1] * q[0] + c1[1] * q[1]];
                        (v[0] * v[0] + v[1] * v[1])
                            * ((4. / 64.) * (tess::PRECISION * tess::PRECISION))
                    }
                    _ => 1.,
                };
                max_n4 = max_n4.max(n4.min(1048576.));
                tess::curve_patches(verb, &pts, &vx, &mut patches);
                last = *pts.last().unwrap();
                sum[0] += last[0];
                sum[1] += last[1];
                count += 1;
            }
        }
    }
    flush(first, last, sum, count, &mut patches, &mut output);
    // wangs_formula::nextlog16 and FixedCountWedges::IndexCount.
    let log2 = if max_n4 <= 1. {
        0
    } else {
        ((max_n4.to_bits() + 0x7fffff) >> 23) - 127
    };
    let vertex_count = 3 * (1u32 << ((log2 + 3) >> 2));
    Geometry::Fill {
        vertex_count,
        patches: output,
        winding,
        even_odd,
    }
}

pub(super) fn stroke(path: &SkPath, m: &Mat, params: StrokeParams, cap: Cap) -> Geometry {
    let (patches, n4) = stroke::stroke_patches_with_tolerance(path, m, cap);
    let max_scale = stroke::svd_max(m.sx, m.kx, m.ky, m.sy, m);
    // LinearTolerances::requiredStrokeEdges / FixedCountStrokes::VertexCount.
    let radius = if params.radius == 0. {
        0.5
    } else {
        params.radius * max_scale
    };
    let per_rad = 0.5 / (1. - (1. / tess::PRECISION) / radius).max(-1.).acos();
    let radial = (per_rad * core::f32::consts::PI).ceil().max(1.) as u32;
    let parametric = n4.sqrt().sqrt().ceil() as u32;
    let joins = if params.join > 0. {
        4
    } else if params.join == 0. {
        3
    } else {
        2 + radial
    };
    let vertex_count = (joins + radial + parametric).min(16383) * 2;
    Geometry::Stroke {
        vertex_count,
        patches: patches
            .iter()
            .map(|patch| {
                let p = patch.p;
                [
                    [p[0][0], p[0][1], p[1][0], p[1][1]],
                    [
                        p[2][0],
                        p[2][1],
                        p[3][0],
                        if patch.conic { 0. } else { p[3][1] },
                    ],
                    [m.sx, m.ky, m.kx, m.sy],
                    [m.tx, m.ty, max_scale, f32::from(patch.conic)],
                    [patch.join[0], patch.join[1], params.radius, params.join],
                ]
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_allocator_counts_include_preconversion_and_deferred_tolerances() {
        // counts.cpp, independently built against pinned Skia. In particular,
        // recomputing Wang's formula on the emitted line/quad cubics fails.
        let stroke_counts = [
            [[26, 48, 50, 54], [18, 40, 42, 46], [20, 42, 44, 48]],
            [[50, 102, 104, 112], [30, 82, 84, 92], [32, 84, 86, 94]],
        ];
        let fill_counts = [[3, 48, 48, 48], [3, 96, 96, 96]];
        for (s, scale) in [1., 5.].into_iter().enumerate() {
            let m = Mat::scale_translate(scale, scale, 0., 0.);
            for kind in 0..4 {
                let mut path = SkPath::new();
                path.move_to([0., 0.]);
                match kind {
                    0 => path.line_to([100., 100.]),
                    1 => path.quad_to([0., 100.], [100., 100.]),
                    2 => path.cubic_to([0., 50.], [50., 100.], [100., 100.]),
                    _ => path.conic_to([0., 100.], [100., 100.], core::f32::consts::FRAC_1_SQRT_2),
                }
                let Geometry::Fill { vertex_count, .. } = fill(&path, &m, false, false) else {
                    panic!()
                };
                assert_eq!(
                    vertex_count, fill_counts[s][kind],
                    "fill scale {scale} kind {kind}"
                );
                for (j, join) in [-1., 0., 2.].into_iter().enumerate() {
                    let Geometry::Stroke { vertex_count, .. } =
                        stroke(&path, &m, StrokeParams { radius: 4.5, join }, Cap::Round)
                    else {
                        panic!()
                    };
                    assert_eq!(
                        vertex_count, stroke_counts[s][j][kind],
                        "stroke scale {scale} kind {kind} join {join}"
                    );
                }
            }
        }
    }
}
