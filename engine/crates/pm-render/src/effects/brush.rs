//! Main effect-path.ts: evaluated typed path/mask geometry and bounded soft
//! brush maximum coverage; hard round strokes use native vector area coverage.
use super::{
    plan::{Binding, EffectStep},
    resources::Asset,
};
use pm_eval::{JsResult, Scope, node};
use pm_model::Node;
use std::sync::Arc;
type Point = [f64; 2];
fn trim(points: &[Point], start: f64, end: f64) -> Vec<Point> {
    let mut lengths = vec![0.];
    for pair in points.windows(2) {
        lengths.push(
            lengths.last().unwrap() + (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]),
        );
    }
    let total = *lengths.last().unwrap();
    let lo = start.clamp(0., 1.) * total;
    let hi = end.clamp(0., 1.) * total;
    let mut out = vec![];
    if hi <= lo {
        return out;
    }
    for (i, pair) in points.windows(2).enumerate() {
        let l = lengths[i];
        let d = lengths[i + 1] - l;
        let from = lo.max(l);
        let to = hi.min(l + d);
        if d <= 0. || to <= from {
            continue;
        }
        let at =
            |x: f64| std::array::from_fn(|c| pair[0][c] + (pair[1][c] - pair[0][c]) * (x - l) / d);
        if out.is_empty() {
            out.push(at(from));
        }
        out.push(at(to));
    }
    out
}
fn affine(x: f64, y: f64, r: f64, sx: f64, sy: f64) -> [f64; 6] {
    let a = r.to_radians();
    [
        a.cos() * sx,
        a.sin() * sx,
        -a.sin() * sy,
        a.cos() * sy,
        x,
        y,
    ]
}
fn apply(m: [f64; 6], p: Point) -> Point {
    [
        m[0] * p[0] + m[2] * p[1] + m[4],
        m[1] * p[0] + m[3] * p[1] + m[5],
    ]
}
pub(crate) fn raster<'d>(
    scope: &mut Scope<'_, 'd>,
    provider: &'d Node,
    path_id: &str,
    time: f64,
    step: &EffectStep,
    size: (u32, u32),
    scale: (f64, f64),
) -> JsResult<Result<Arc<Asset>, String>> {
    let (width, height) = size;
    let value = |key: &str| {
        step.params.iter().find(|p| p.key == key).map_or(
            match key {
                "hardness" | "end" => 100.,
                _ => 0.,
            },
            |p| match p.value {
                Binding::Float(v) => v,
                _ => 0.,
            },
        )
    };
    let (brush_width, hardness, spacing, start, end) = (
        value("brushSize"),
        value("hardness"),
        value("spacing"),
        value("start"),
        value("end"),
    );
    if ![brush_width, hardness, spacing, start, end]
        .iter()
        .all(|v| v.is_finite())
    {
        return Ok(Err("Non-finite stroke brush".into()));
    }
    let masks = node::array(node::member(Some(provider), "masks")).unwrap_or(&[]);
    let selected_mask = masks
        .iter()
        .find(|m| node::to_js_string(node::member(Some(m), "id")).to_string_lossy() == path_id);
    let paths =
        node::array(node::member(node::member(Some(provider), "d"), "paths")).unwrap_or(&[]);
    let selected_path = paths
        .iter()
        .find(|p| node::to_js_string(node::member(Some(p), "id")).to_string_lossy() == path_id);
    if selected_mask.is_none() && selected_path.is_none() {
        return Ok(Err("Missing stroke path".into()));
    }
    let items: Vec<(Option<&Node>, Option<&Node>)> =
        if selected_mask.is_some() && value("allMasks") != 0. {
            masks
                .iter()
                .map(|m| (Some(m), node::member(Some(m), "path")))
                .collect()
        } else {
            vec![(
                selected_mask,
                selected_path.or_else(|| node::member(selected_mask, "path")),
            )]
        };
    let mut curves = vec![];
    let mut count = 0usize;
    for (mask, path) in items {
        let mut path_value = None;
        let mut matrix = [1., 0., 0., 1., 0., 0.];
        if let Some(mask) = mask {
            let id = node::to_js_string(node::member(Some(mask), "id")).to_string_lossy();
            let p = node::member(Some(mask), "p");
            let mut ev = |key: &str| -> JsResult<f64> {
                let v = scope
                    .channel_p(
                        provider,
                        node::member(p, key),
                        time,
                        &format!("m.{id}.{key}"),
                    )?
                    .to_number();
                Ok(if v.is_nan() { 0. } else { v })
            };
            matrix = affine(ev("x")?, ev("y")?, ev("rotation")?, 1., 1.);
        }
        let mut wrapped = None;
        let points = if let Some(path) = path {
            let v = scope.path_values(provider, path, time, "")?.path_values();
            if v.vertices.len() > 2048 {
                return Ok(Err("Stroke geometry exceeds the vertex budget".into()));
            }
            if mask.is_some() {
                matrix = pm_eval::transform::mul(
                    &matrix,
                    &affine(v.x, v.y, v.rotation, v.scale_x / 100., v.scale_y / 100.),
                );
            } else {
                matrix = scope.detached_group_matrix(provider, path, time)?;
            }
            let mut points = pm_vector::path_points(&v)
                .into_iter()
                .map(|p| [p.x, p.y])
                .collect::<Vec<_>>();
            let lo = (v.trim_start.clamp(0., 100.) / 100. + (v.trim_offset / 360.).rem_euclid(1.))
                .rem_euclid(1.);
            let span = (v.trim_end.clamp(0., 100.) - v.trim_start.clamp(0., 100.)) / 100.;
            if span < 0.999999 {
                if span <= 0. {
                    points.clear();
                } else if lo + span <= 1. {
                    points = trim(&points, lo, lo + span);
                } else {
                    wrapped = Some(trim(&points, 0., lo + span - 1.));
                    points = trim(&points, lo, 1.);
                }
            }
            path_value = Some(v);
            points
        } else {
            let mask = mask.unwrap();
            let p = node::member(Some(mask), "p");
            let id = node::to_js_string(node::member(Some(mask), "id")).to_string_lossy();
            let w = scope
                .channel_p(provider, node::member(p, "w"), time, &format!("m.{id}.w"))?
                .to_number()
                / 2.;
            let h = scope
                .channel_p(provider, node::member(p, "h"), time, &format!("m.{id}.h"))?
                .to_number()
                / 2.;
            if scope
                .evaluated(
                    provider,
                    node::member(Some(mask), "shape"),
                    time,
                    &format!("m.{id}.shape"),
                )?
                .to_js_string()
                .to_string_lossy()
                == "ellipse"
            {
                (0..129)
                    .map(|i| {
                        let a = i as f64 * std::f64::consts::PI / 64.;
                        [a.cos() * w.abs(), a.sin() * h.abs()]
                    })
                    .collect()
            } else {
                vec![[-w, -h], [w, -h], [w, h], [-w, h], [-w, -h]]
            }
        };
        let copies = path_value
            .as_ref()
            .map_or(1, |v| (v.copies + 0.5).floor().clamp(1., 256.) as usize);
        for i in 0..copies {
            count += points.len();
            if count > 65536 {
                return Ok(Err("Stroke geometry exceeds the segment budget".into()));
            }
            let m = path_value.as_ref().map_or(matrix, |v| {
                pm_eval::transform::mul(
                    &matrix,
                    &affine(
                        i as f64 * v.repeat_x,
                        i as f64 * v.repeat_y,
                        i as f64 * v.repeat_rotation,
                        1.,
                        1.,
                    ),
                )
            });
            curves.push(points.iter().map(|&p| apply(m, p)).collect::<Vec<_>>());
            if let Some(wrapped) = &wrapped {
                curves.push(wrapped.iter().map(|&p| apply(m, p)).collect());
            }
        }
    }
    let world = scope.world_matrix(provider, time)?;
    let matrix = [
        world[0] * scale.0,
        world[1] * scale.1,
        world[2] * scale.0,
        world[3] * scale.1,
        world[4] * scale.0,
        world[5] * scale.1,
    ];
    Ok(soft_brush(
        curves,
        matrix,
        width,
        height,
        brush_width,
        hardness,
        spacing,
        start,
        end,
        value("sequential") != 0.,
    )
    .map(Arc::new))
}
#[allow(clippy::too_many_arguments)]
fn soft_brush(
    curves: Vec<Vec<Point>>,
    matrix: [f64; 6],
    width: u32,
    height: u32,
    brush_width: f64,
    hardness: f64,
    spacing: f64,
    start: f64,
    end: f64,
    sequential: bool,
) -> Result<Asset, String> {
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .ok_or("Stroke allocation overflow")?;
    if pixels > 64 * 1024 * 1024 {
        return Err("Stroke brush exceeds the raster allocation budget".into());
    }
    let mut alpha = vec![0u8; pixels];
    let radius = (brush_width / 2.).max(0.);
    let lengths = curves
        .iter()
        .map(|p| {
            p.windows(2)
                .map(|p| (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1]))
                .sum::<f64>()
        })
        .collect::<Vec<_>>();
    let total = lengths.iter().sum::<f64>();
    let mut offset = 0.;
    let paths = curves
        .iter()
        .zip(&lengths)
        .map(|(points, &length)| {
            let d = if length == 0. { 1. } else { length };
            let lo = if sequential {
                (start / 100. * total - offset) / d
            } else {
                start / 100.
            };
            let hi = if sequential {
                (end / 100. * total - offset) / d
            } else {
                end / 100.
            };
            offset += length;
            trim(points, lo, hi)
        })
        .collect::<Vec<_>>();
    if matrix.iter().any(|v| !v.is_finite())
        || paths.iter().flatten().flatten().any(|v| !v.is_finite())
    {
        return Err("Non-finite stroke geometry".into());
    }
    if hardness >= 100. && spacing <= 0. {
        use pm_vector::{
            kurbo::{Affine, BezPath, Cap, Join, Stroke},
            render::scene::{CoverageItem, CoverageScene, ItemStyle, PixelRect},
        };
        let mut scene = CoverageScene::default();
        if radius > 0. && end > start {
            for points in paths {
                let mut path = BezPath::new();
                if let Some(first) = points.first() {
                    path.move_to((first[0], first[1]));
                    for p in &points[1..] {
                        path.line_to((p[0], p[1]));
                    }
                }
                scene.items.push(CoverageItem {
                    path,
                    transform: Affine::new(matrix),
                    style: ItemStyle::Stroke(
                        Stroke::new(brush_width)
                            .with_caps(Cap::Round)
                            .with_join(Join::Round),
                    ),
                    color: [1.; 4],
                });
            }
        }
        let vector = crate::vector::VectorSource::Scene {
            scene,
            rect: PixelRect {
                x: 0,
                y: 0,
                width,
                height,
            },
        };
        return Ok(Asset {
            key: vector.key(),
            width,
            height,
            rgba: vec![],
            lut: None,
            vector: Some(vector),
            brush: true,
        });
    }
    if radius > 0. && end > start {
        let determinant = matrix[0] * matrix[3] - matrix[1] * matrix[2];
        if determinant.abs() >= 1e-12 {
            let inv = pm_eval::transform::invert(&matrix).unwrap();
            let antialias = 0.5 * inv[0].hypot(inv[1]).max(inv[2].hypot(inv[3]));
            let pad_x = radius * matrix[0].hypot(matrix[2]) + 1.;
            let pad_y = radius * matrix[1].hypot(matrix[3]) + 1.;
            let inner = radius * (hardness / 100.).clamp(0., 1.);
            let mut work = 0usize;
            let mut draw = |a: Point, b: Point| -> Result<(), String> {
                let aa = apply(matrix, a);
                let bb = apply(matrix, b);
                let x0 = (aa[0].min(bb[0]) - pad_x).floor().clamp(0., width as f64) as u32;
                let x1 = (aa[0].max(bb[0]) + pad_x).ceil().clamp(0., width as f64) as u32;
                let y0 = (aa[1].min(bb[1]) - pad_y).floor().clamp(0., height as f64) as u32;
                let y1 = (aa[1].max(bb[1]) + pad_y).ceil().clamp(0., height as f64) as u32;
                work += (x1 - x0) as usize * (y1 - y0) as usize;
                if work > 64 * 1024 * 1024 {
                    return Err("Stroke brush exceeds the raster work budget".into());
                }
                let dx = b[0] - a[0];
                let dy = b[1] - a[1];
                let length = dx * dx + dy * dy;
                for y in y0..y1 {
                    for x in x0..x1 {
                        let p = apply(inv, [x as f64 + 0.5, y as f64 + 0.5]);
                        let t = if length == 0. {
                            0.
                        } else {
                            ((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / length
                        }
                        .clamp(0., 1.);
                        let d = (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy);
                        let a = (((radius + antialias - d)
                            / (2. * antialias).max(radius - inner + antialias))
                        .clamp(0., 1.)
                            * 255.
                            + 0.5)
                            .floor() as u8;
                        let at = y as usize * width as usize + x as usize;
                        alpha[at] = alpha[at].max(a);
                    }
                }
                Ok(())
            };
            for points in paths {
                if spacing <= 0. {
                    for p in points.windows(2) {
                        draw(p[0], p[1])?;
                    }
                } else {
                    let step = (brush_width * spacing / 100.).max(0.25);
                    let mut distance = 0.;
                    let mut next = 0.;
                    let mut dabs = 0;
                    for p in points.windows(2) {
                        let length = (p[1][0] - p[0][0]).hypot(p[1][1] - p[0][1]);
                        if length == 0. {
                            continue;
                        }
                        while next <= distance + length {
                            dabs += 1;
                            if dabs > 65536 {
                                return Err("Stroke brush exceeds the dab budget".into());
                            }
                            let t = (next - distance) / length;
                            let point = std::array::from_fn(|i| p[0][i] + t * (p[1][i] - p[0][i]));
                            draw(point, point)?;
                            next += step;
                        }
                        distance += length;
                    }
                }
            }
        }
    }
    use sha2::{Digest, Sha256};
    let key = Sha256::digest(&alpha).into();
    let mut rgba = Vec::with_capacity(pixels * 4);
    for row in alpha.chunks_exact(width as usize).rev() {
        for &a in row {
            let a = a as f32 / 255.;
            rgba.extend([a, a, a, a]);
        }
    }
    Ok(Asset {
        key,
        width,
        height,
        rgba,
        lut: None,
        vector: None,
        brush: true,
    })
}
