//! Independent f64 reference for text scene coverage and paint.
//!
//! The scene supplies shaped outline paths, their exact device transforms,
//! authored colors and draw order. No engine coverage, atlas, raster, or
//! compositing output enters this calculation. `pm-reference` integrates
//! each outline at 32 cells per pixel side and composites in f64.

use kurbo::{Affine, Point, Shape, StrokeOpts};
use pm_protocol::color::WorkingEncoding;
use pm_reference::compare::{Class, classify};
use pm_reference::composite::{Operator, porter_duff};
use pm_reference::coverage::{FillRule, fill};
use pm_reference::gaussian::{Edge, blur};
use pm_text::render::scene::{ImagePixels, ItemContent, PathStyle, TextScene};

pub struct TextReference {
    pub rgba: Vec<f64>,
    pub classes: Vec<Class>,
    pub image_pixels: usize,
    /// Only opaque WorkingF32 inputs without an authored paint program remain
    /// unreferenced; COLR programs are interpreted independently in f64.
    pub unreferenced: usize,
}

pub fn render(scene: &TextScene) -> TextReference {
    render_in_working(scene, WorkingEncoding::Standard)
}

/// The same independent integration in the project's compositing domain.
pub fn render_in_working(scene: &TextScene, working: WorkingEncoding) -> TextReference {
    render_with_samples(scene, working, 32)
}

/// Independent integration at the selected subpixel grid. Whole-image
/// refinement verifies convergence before a close pixel or geometry verdict.
pub fn render_with_samples(
    scene: &TextScene,
    working: WorkingEncoding,
    samples: usize,
) -> TextReference {
    assert!(samples >= 32);
    let (w, h) = (scene.width as usize, scene.height as usize);
    let mut rgba = vec![0.0; w * h * 4];
    let mut edge = vec![false; w * h];
    let mut resampled = vec![false; w * h];
    let mut unreferenced = vec![false; w * h];
    for item in &scene.items {
        assert!(item.blur.is_finite() && item.blur >= 0.0, "text blur sigma");
        // Keep the full off-canvas source footprint. Cropping to the target
        // before convolution would lose ink that blurs into its edge.
        let reach = item.bounds.inflate(
            (8.0 * item.blur).ceil() + 2.0,
            (8.0 * item.blur).ceil() + 2.0,
        );
        let (wx, wy) = (reach.x0.floor() as i64, reach.y0.floor() as i64);
        let (x1, y1) = (reach.x1.ceil() as i64, reach.y1.ceil() as i64);
        if x1 <= 0 || y1 <= 0 || wx >= w as i64 || wy >= h as i64 {
            continue;
        }
        let (ew, eh) = ((x1 - wx) as usize, (y1 - wy) as usize);
        if ew == 0 || eh == 0 {
            continue;
        }
        let shift = Affine::translate((-(wx as f64), -(wy as f64)));
        let mut src = vec![0.0; ew * eh * 4];
        let a = item.color[3] * item.opacity;
        match &item.content {
            ItemContent::Coverage { paths } => {
                let mut mask = vec![0.0; ew * eh];
                for p in paths {
                    let local = match &p.style {
                        PathStyle::Fill => p.path.as_ref().clone(),
                        PathStyle::Stroke(stroke) => {
                            let c = p.transform.as_coeffs();
                            let scale = c[0].hypot(c[1]).max(c[2].hypot(c[3])).max(1e-9);
                            kurbo::stroke(
                                p.path.iter(),
                                stroke,
                                &StrokeOpts::default(),
                                1e-5 / scale,
                            )
                        }
                    };
                    let outline = shift * p.transform * local;
                    let coverage = fill(&outline, FillRule::NonZero, samples, ew, eh);
                    // One glyph/path is an authored draw. Its coverage is
                    // composited over previous paths; merging outlines
                    // first would cancel or overfill overlapping glyphs.
                    for y in coverage.y0..coverage.y0 + coverage.height {
                        for x in coverage.x0..coverage.x0 + coverage.width {
                            let k = coverage.at(x, y);
                            let dst = &mut mask[y * ew + x];
                            *dst = k + *dst * (1.0 - k);
                        }
                    }
                }
                let paint = [
                    channel(item.color[0], working) * a,
                    channel(item.color[1], working) * a,
                    channel(item.color[2], working) * a,
                    a,
                ];
                for y in 0..eh {
                    for x in 0..ew {
                        let k = mask[y * ew + x];
                        let (gx, gy) = (wx + x as i64, wy + y as i64);
                        // Classify the boundary of this item's own path
                        // mask before later opaque items can hide it in the
                        // completed alpha image. An exactly pixel-aligned
                        // outline has only 0/1 coverage, so fractional
                        // coverage alone cannot identify its edge.
                        let support_boundary = k > 0.0
                            && (x == 0
                                || mask[y * ew + x - 1] == 0.0
                                || x + 1 == ew
                                || mask[y * ew + x + 1] == 0.0
                                || y == 0
                                || mask[(y - 1) * ew + x] == 0.0
                                || y + 1 == eh
                                || mask[(y + 1) * ew + x] == 0.0);
                        if gx >= 0
                            && gy >= 0
                            && gx < w as i64
                            && gy < h as i64
                            && a > 0.0
                            && ((k > 0.0 && k < 1.0) || support_boundary)
                        {
                            edge[gy as usize * w + gx as usize] = true;
                        }
                        for c in 0..4 {
                            src[(y * ew + x) * 4 + c] = paint[c] * k;
                        }
                    }
                }
            }
            ItemContent::Image { image, transform } => {
                let colr = match image {
                    ImagePixels::Colr(program) => {
                        Some(super::colr_reference::raster(program, working, samples))
                    }
                    _ => None,
                };
                let b = item.bounds.inflate(1.0, 1.0);
                let (x0, y0) = (
                    (b.x0.floor() as i64 - wx).max(0) as usize,
                    (b.y0.floor() as i64 - wy).max(0) as usize,
                );
                let (x1, y1) = (
                    (b.x1.ceil() as i64 - wx).clamp(0, ew as i64) as usize,
                    (b.y1.ceil() as i64 - wy).clamp(0, eh as i64) as usize,
                );
                for y in y0..y1 {
                    for x in x0..x1 {
                        let (gx, gy) = (wx + x as i64, wy + y as i64);
                        let pixel = image_sample(
                            image,
                            colr.as_deref(),
                            *transform,
                            gx as f64,
                            gy as f64,
                            working,
                        );
                        for c in 0..4 {
                            src[(y * ew + x) * 4 + c] = pixel[c] * a;
                        }
                        if gx >= 0 && gy >= 0 && gx < w as i64 && gy < h as i64 {
                            let i = gy as usize * w + gx as usize;
                            if matches!(image, ImagePixels::WorkingF32(_)) {
                                unreferenced[i] = true;
                            } else {
                                resampled[i] = true;
                            }
                        }
                    }
                }
            }
        }
        if item.blur > 0.0 {
            src = blur(&src, ew, eh, 4, (item.blur, item.blur), Edge::Transparent);
        }
        for y in 0..eh {
            let gy = wy + y as i64;
            if gy < 0 || gy >= h as i64 {
                continue;
            }
            for x in 0..ew {
                let gx = wx + x as i64;
                if gx < 0 || gx >= w as i64 {
                    continue;
                }
                let i = gy as usize * w + gx as usize;
                let s = &src[(y * ew + x) * 4..][..4];
                if s[3] <= 0.0 {
                    continue;
                }
                if matches!(
                    &item.content,
                    ItemContent::Image {
                        image: ImagePixels::WorkingF32(_),
                        ..
                    }
                ) {
                    unreferenced[i] = true;
                }
                if item.blur > 0.0 && s[3] < a - 1e-12 {
                    edge[i] = true;
                }
                let dst = &mut rgba[i * 4..][..4];
                let result = porter_duff(
                    Operator::SourceOver,
                    [s[0], s[1], s[2], s[3]],
                    [dst[0], dst[1], dst[2], dst[3]],
                );
                dst.copy_from_slice(&result);
            }
        }
    }
    let image_pixels = resampled.iter().filter(|v| **v).count();
    let mut classes = classify(&edge, &resampled, w, h, 2);
    let unknown = pm_reference::compare::dilate(&unreferenced, w, h, 2);
    let unreferenced = unknown.iter().filter(|v| **v).count();
    for (i, missing) in unknown.into_iter().enumerate() {
        if missing {
            classes[i] = Class::Unreferenced;
            rgba[i * 4..i * 4 + 4].fill(f64::NAN);
        }
    }
    TextReference {
        rgba,
        classes,
        image_pixels,
        unreferenced,
    }
}

fn channel(encoded: f64, working: WorkingEncoding) -> f64 {
    if working == WorkingEncoding::LinearLight {
        pm_reference::color::Transfer::Srgb.to_linear(encoded)
    } else {
        encoded
    }
}

fn texel(
    image: &ImagePixels,
    colr: Option<&[f64]>,
    x: i64,
    y: i64,
    working: WorkingEncoding,
) -> [f64; 4] {
    let (w, h) = image.dimensions();
    if x < 0 || y < 0 || x >= i64::from(w) || y >= i64::from(h) {
        return [0.0; 4];
    }
    let i = (y as usize * w as usize + x as usize) * 4;
    match image {
        ImagePixels::Rgba8(v) => {
            let a = f64::from(v.data[i + 3]) / 255.0;
            std::array::from_fn(|c| {
                let e = f64::from(v.data[i + c]) / 255.0;
                if c < 3 && a > 0.0 {
                    channel(e / a, working) * a
                } else {
                    e
                }
            })
        }
        // The COLR graph produced these working pixels in production. They
        // are opaque compositing inputs here, never independent truth.
        ImagePixels::WorkingF32(v) => std::array::from_fn(|c| f64::from(v.data[i + c])),
        ImagePixels::Colr(_) => {
            let pixels = colr.expect("COLR reference is integrated before sampling");
            pixels[i..i + 4].try_into().unwrap()
        }
    }
}

fn bilinear(
    image: &ImagePixels,
    colr: Option<&[f64]>,
    p: Point,
    working: WorkingEncoding,
) -> [f64; 4] {
    let (u, v) = (p.x - 0.5, p.y - 0.5);
    let (x, y) = (u.floor() as i64, v.floor() as i64);
    let (fx, fy) = (u - x as f64, v - y as f64);
    let taps = [
        (x, y, (1.0 - fx) * (1.0 - fy)),
        (x + 1, y, fx * (1.0 - fy)),
        (x, y + 1, (1.0 - fx) * fy),
        (x + 1, y + 1, fx * fy),
    ];
    let mut out = [0.0; 4];
    for (x, y, weight) in taps {
        let p = texel(image, colr, x, y, working);
        for c in 0..4 {
            out[c] += p[c] * weight;
        }
    }
    out
}

/// Clip a convex polygon to one side of an axis-aligned line.
fn clip(poly: &[(f64, f64)], axis: usize, bound: f64, keep_above: bool) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    if poly.is_empty() {
        return out;
    }
    let inside = |p: (f64, f64)| {
        let v = if axis == 0 { p.0 } else { p.1 };
        if keep_above { v >= bound } else { v <= bound }
    };
    for i in 0..poly.len() {
        let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
        let (av, bv) = if axis == 0 { (a.0, b.0) } else { (a.1, b.1) };
        if inside(a) != inside(b) {
            let t = (bound - av) / (bv - av);
            out.push((a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1)));
        }
        if inside(b) {
            out.push(b);
        }
    }
    out
}

fn area(poly: &[(f64, f64)]) -> f64 {
    if poly.len() < 3 {
        return 0.0;
    }
    let sum: f64 = (0..poly.len())
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum();
    sum.abs() * 0.5
}

/// Exact box-area reconstruction of a downscaled premultiplied image,
/// including rotated/sheared texels, by clipping the inverse output pixel
/// against source texel squares. Upscaling uses centered bilinear samples.
fn image_sample(
    image: &ImagePixels,
    colr: Option<&[f64]>,
    transform: Affine,
    x: f64,
    y: f64,
    working: WorkingEncoding,
) -> [f64; 4] {
    let inv = transform.inverse();
    let sample = |x: f64, y: f64| {
        let p = inv * Point::new(x, y);
        (p.x, p.y)
    };
    let [a, b, c, d, _, _] = inv.as_coeffs();
    let xx = a * a + c * c;
    let xy = a * b + c * d;
    let yy = b * b + d * d;
    let major = ((xx + yy + ((xx - yy).powi(2) + 4.0 * xy * xy).sqrt()) * 0.5).sqrt();
    if major <= 1.0 {
        return bilinear(image, colr, inv * Point::new(x + 0.5, y + 0.5), working);
    }
    let quad = [
        sample(x, y),
        sample(x + 1.0, y),
        sample(x + 1.0, y + 1.0),
        sample(x, y + 1.0),
    ];
    let whole = area(&quad);
    if whole <= 0.0 {
        return [0.0; 4];
    }
    let x0 = quad
        .iter()
        .map(|p| p.0)
        .fold(f64::INFINITY, f64::min)
        .floor() as i64;
    let x1 = quad
        .iter()
        .map(|p| p.0)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil() as i64;
    let y0 = quad
        .iter()
        .map(|p| p.1)
        .fold(f64::INFINITY, f64::min)
        .floor() as i64;
    let y1 = quad
        .iter()
        .map(|p| p.1)
        .fold(f64::NEG_INFINITY, f64::max)
        .ceil() as i64;
    let mut out = [0.0; 4];
    for sy in y0.max(0)..y1.min(i64::from(image.dimensions().1)) {
        for sx in x0.max(0)..x1.min(i64::from(image.dimensions().0)) {
            let mut p = quad.to_vec();
            p = clip(&p, 0, sx as f64, true);
            p = clip(&p, 0, sx as f64 + 1.0, false);
            p = clip(&p, 1, sy as f64, true);
            p = clip(&p, 1, sy as f64 + 1.0, false);
            let weight = area(&p) / whole;
            let texel = texel(image, colr, sx, sy, working);
            for c in 0..4 {
                out[c] += texel[c] * weight;
            }
        }
    }
    out
}
