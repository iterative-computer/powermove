//! G-V3, the SVG image gate (docs/engine/gate-policy.md; docs/engine/vector.md,
//! "SVG images"). Every SVG image is drawn at output resolution; the gate
//! proves it against a reference and against today's look:
//!
//! - **Reference.** Documents of plain fills and strokes: exact 16 × 16
//!   coverage (`pm_reference::coverage::fill`) of their geometry, written
//!   here in kurbo independently of usvg, composited in f64. Paint
//!   pm-reference does not model (gradients, clip paths, masks, group
//!   opacity, text): resvg, a renderer independent of the engine's Vello
//!   painter, at 16 × 16 supersampling box-filtered down, so its coverage
//!   is exact to 1/256 per pixel and its paint is evaluated per subsample.
//! - **Today.** The legacy compositor's image quad over today's bucket
//!   raster: `svgRasterDimensions`' bucket (quarter-step density, ceil),
//!   rasterized (resvg stands in for Chromium's raster step; the bucket
//!   rasters' only measured difference is edge antialiasing), sampled
//!   bilinearly with clamped edges through the box's `fit` window, the
//!   quad's own edges single-sampled.
//! - **Rule.** Interiors (2 px from any edge) within 1 CV of today; edges
//!   at least as close to the reference as today (max, p99.9, mean, and per
//!   pixel within 1 CV for 99.9% and 4 CV for any: `SVG_BOUNDS`, the
//!   engine stores Vello's 8-bit output); geometry (alpha area and
//!   centroid) within 1e-3 of the reference plus the 8-bit storage bound
//!   (`geometry_tolerance`); preview equals export.
//! - **Sharpness under scale** is the same comparison over 25% … 800% and
//!   an animated scale, printed (and written to `PM_SVG_EVIDENCE/gate.json`
//!   when set).
//! - **Semantics** (V-A2): the triage's decisions per document class, and,
//!   when the Chromium references of the 141 probe documents are present
//!   (`svg-gate-*` capture, as `corpus.rs` finds captures), every document's
//!   interior colours against Chromium's.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_reference::compare::{Bounds, Class, classify, compare, geometry, identical};
use pm_reference::coverage::{FillRule, fill};
use pm_render::vector::svg::document;
use pm_render::{MapResolver, RenderRequest, Renderer};
use pm_vector::kurbo::{
    Affine, BezPath, Cap, Circle, Join, Point, Rect, RoundedRect, Shape, Stroke, StrokeOpts, stroke,
};
use pm_vector::render::scene::PixelRect;
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

const W: u32 = 192;
const H: u32 = 144;

/// The gate's bounds (gate-policy.md, "a gate with other storage states
/// its own"): Vello stores 8-bit straight colour, premultiplied after, so a
/// premultiplied channel carries up to half a code from the colour and half
/// from the alpha (1 CV), where today's canvas stores premultiplied 8-bit
/// (0.5). Interiors and the per-pixel rule are `Bounds::EIGHT_BIT`'s.
const SVG_BOUNDS: Bounds = Bounds {
    precision_cv: 1.0,
    ..Bounds::EIGHT_BIT
};

/// Cases measured to fail the per-pixel rule, and why (reported, never
/// passed by a wider bound). Each must still fail, and only that way: a
/// fix moves it out of this list.
///
/// - `art`: gradient pixels up to 2.75 CV further from the reference than
///   today (p99.9 about 2; interiors up to 1.1 CV from today), while the
///   mean is closer. Vello evaluates
///   gradients through an 8-bit ramp of 512 entries (nearest), and stores
///   8-bit colour in its layers and output; today's raster computes each
///   pixel's gradient in f32. Float paint for gradients is the fix.
/// - `shapes` above 200%: usvg turns arcs (`<circle>`, `<ellipse>`, `rx`,
///   path `A`) into cubics within 0.1 user units, so a scaled-up circle is
///   off its true outline by 0.03% of its radius (0.04 px at 800% here,
///   12 CV on edge pixels). Blink draws arcs as conics. The fix is usvg's
///   tolerance (a one-constant patch), an orchestrator decision.
const PENDING: &[(&str, &str)] = &[("art", "per pixel"), ("art", " CV ")];
const PENDING_SCALED: &[(&str, &str)] = &[
    ("art", "per pixel"),
    ("art", " CV "),
    ("shapes", "per pixel"),
    ("shapes", "interior"),
];

/// Splits gate failures into the unexpected ones and the pending cases'.
fn settle(failures: Vec<String>, pending_cases: &[(&str, &str)]) -> Vec<String> {
    let (pending, unexpected): (Vec<_>, Vec<_>) = failures.into_iter().partition(|f| {
        pending_cases
            .iter()
            .any(|(case, kind)| f.starts_with(&format!("{case} ")) && f.contains(kind))
    });
    let mut cases: Vec<&str> = pending_cases.iter().map(|(c, _)| *c).collect();
    cases.dedup();
    for case in cases {
        let n = pending.iter().filter(|f| f.starts_with(&format!("{case} "))).count();
        eprintln!("pending {case}: {n} measured failures");
        assert!(n > 0, "{case} passes now: take it out of PENDING");
    }
    unexpected
}

/// A temporary directory, removed when dropped.
struct Dir(std::path::PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("pm-render-svg-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A placement of an SVG image layer: the box, `fit`, scale (percent) and
/// rotation (degrees) about the frame's center.
#[derive(Clone, Copy, Debug)]
struct Place {
    fit: &'static str,
    bw: f64,
    bh: f64,
    scale: f64,
    rotation: f64,
}

impl Place {
    fn world(&self) -> Affine {
        Affine::translate((f64::from(W) / 2.0, f64::from(H) / 2.0))
            * Affine::rotate(self.rotation.to_radians())
            * Affine::scale(self.scale / 100.0)
    }
}

/// An SVG document under test: its source, natural size (the asset
/// record), and, for plain fills and strokes, its exact paint.
struct Case {
    name: &'static str,
    svg: &'static str,
    size: (f64, f64),
    /// Painted in order: premultiplied sRGB colour and filled outline (user
    /// units). `None`: the supersampled resvg reference.
    plain: Option<fn() -> Vec<([f64; 4], BezPath, FillRule)>>,
    /// Cells per pixel of the exact coverage: stroke outlines overlap at
    /// their joins, where `coverage::fill` is off by up to 1/n per boundary
    /// segment, so they take 64 (4 CV per segment).
    cells: usize,
}

const SHAPES: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="48" viewBox="0 0 80 48"><rect x="4.5" y="6" width="30" height="20.25" fill="#3FCF8E"/><circle cx="56" cy="24" r="17.5" fill="#4C8DFF"/><rect x="10" y="30" width="60" height="14" rx="6" fill="#FFD66B" fill-opacity="0.75"/></svg>"##;

fn shapes() -> Vec<([f64; 4], BezPath, FillRule)> {
    let rgb = |hex: u32, a: f64| {
        let c = |s: u32| f64::from((hex >> s) & 0xff) / 255.0 * a;
        [c(16), c(8), c(0), a]
    };
    vec![
        (rgb(0x3FCF8E, 1.0), Rect::new(4.5, 6.0, 34.5, 26.25).to_path(1e-4), FillRule::NonZero),
        (rgb(0x4C8DFF, 1.0), Circle::new((56.0, 24.0), 17.5).to_path(1e-4), FillRule::NonZero),
        (
            rgb(0xFFD66B, 0.75),
            RoundedRect::new(10.0, 30.0, 70.0, 44.0, 6.0).to_path(1e-4),
            FillRule::NonZero,
        ),
    ]
}

const STROKES: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="40"><path d="M6 34 L20 6 L34 30 L46 8 L58 34" fill="none" stroke="#E8E2CF" stroke-width="3.5" stroke-linejoin="round" stroke-linecap="round"/><rect x="8" y="8" width="16" height="10" fill="none" stroke="#FF6B1A" stroke-width="2"/></svg>"##;

fn strokes() -> Vec<([f64; 4], BezPath, FillRule)> {
    let mut zig = BezPath::new();
    zig.move_to((6.0, 34.0));
    for p in [(20.0, 6.0), (34.0, 30.0), (46.0, 8.0), (58.0, 34.0)] {
        zig.line_to(p);
    }
    let style = |w: f64, join: Join, cap: Cap| Stroke::new(w).with_join(join).with_caps(cap);
    let outline = |p: &BezPath, s: &Stroke| stroke(p.iter(), s, &StrokeOpts::default(), 1e-4);
    let rect = Rect::new(8.0, 8.0, 24.0, 18.0).to_path(1e-4);
    vec![
        (
            [0xE8 as f64 / 255.0, 0xE2 as f64 / 255.0, 0xCF as f64 / 255.0, 1.0],
            outline(&zig, &style(3.5, Join::Round, Cap::Round)),
            FillRule::NonZero,
        ),
        (
            [1.0, 0x6B as f64 / 255.0, 0x1A as f64 / 255.0, 1.0],
            outline(&rect, &style(2.0, Join::Miter, Cap::Butt)),
            FillRule::NonZero,
        ),
    ]
}

/// Gradients, a rounded clip, path and group opacity, dashes.
const ART: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="54" viewBox="0 0 320 180"><defs><linearGradient id="a" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#1B2A4A"/><stop offset="0.6" stop-color="#4C8DFF"/><stop offset="1" stop-color="#FFD66B"/></linearGradient><radialGradient id="b" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#FFF4D0"/><stop offset="1" stop-color="#FF6B1A" stop-opacity="0"/></radialGradient><clipPath id="c"><rect x="8" y="8" width="304" height="164" rx="18"/></clipPath></defs><g clip-path="url(#c)"><rect width="320" height="180" fill="url(#a)"/><circle cx="230" cy="70" r="46" fill="url(#b)"/><path d="M0 150 C 60 110, 110 170, 170 130 S 270 100, 320 140 L 320 180 L 0 180 Z" fill="#0C0D12" opacity="0.85"/><g opacity="0.7" transform="translate(70 60) rotate(-12)"><rect x="-30" y="-18" width="60" height="36" rx="6" fill="none" stroke="#E8E2CF" stroke-width="4" stroke-dasharray="10 6"/></g></g></svg>"##;

/// A luminance mask over a radial gradient, crossed strokes.
const MASKED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="60" viewBox="0 0 200 200"><defs><radialGradient id="g" cx="100" cy="100" r="90" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#7CF3C1"/><stop offset="1" stop-color="#14605A"/></radialGradient><mask id="m"><rect width="200" height="200" fill="#000"/><circle cx="100" cy="100" r="80" fill="#fff"/><circle cx="100" cy="100" r="40" fill="#555"/></mask></defs><g mask="url(#m)"><rect width="200" height="200" fill="url(#g)"/><path d="M0 0 L200 200 M200 0 L0 200" stroke="#0B1F1D" stroke-width="12"/></g></svg>"##;

const TEXT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="30"><text x="4" y="22" font-family="Helvetica" font-size="18" fill="#4C8DFF">Pm Svg</text></svg>"##;

fn cases() -> Vec<Case> {
    vec![
        Case { name: "shapes", svg: SHAPES, size: (80.0, 48.0), plain: Some(shapes), cells: 16 },
        Case { name: "strokes", svg: STROKES, size: (64.0, 40.0), plain: Some(strokes), cells: 64 },
        Case { name: "art", svg: ART, size: (96.0, 54.0), plain: None, cells: 16 },
        Case { name: "masked", svg: MASKED, size: (60.0, 60.0), plain: None, cells: 16 },
        Case { name: "text", svg: TEXT, size: (80.0, 30.0), plain: None, cells: 16 },
    ]
}

fn project_for(case: &Case, place: &Place, dir: &Dir) -> (pm_model::Document, MapResolver) {
    let path = dir.0.join(format!("{}.svg", case.name));
    std::fs::write(&path, case.svg).unwrap();
    let mut l = layer(
        "svg",
        "image",
        json!({ "asset": "art", "fit": place.fit, "w": place.bw, "h": place.bh }),
        f64::from(W) / 2.0,
        f64::from(H) / 2.0,
        2.0,
    );
    set(&mut l, "scale.x", json!(place.scale));
    set(&mut l, "scale.y", json!(place.scale));
    set(&mut l, "rotation", json!(place.rotation));
    let mut doc: Value = project(W, H, 2.0, no_fill(), vec![l]);
    asset(&mut doc, "art", "image", "svg", case.size.0 as u32, case.size.1 as u32, 0.0);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("art".into(), path);
    (load(&doc), resolver)
}

/// The engine's frame (premultiplied, 0…1), as a preview or an export.
fn engine(renderer: &mut Renderer, doc: &pm_model::Document, resolver: &MapResolver, export: bool) -> Vec<f32> {
    let mut request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    request.export = export;
    let frame = renderer
        .render(doc, "Ptest-root", ticks(0.0), &request, resolver)
        .expect("render");
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    renderer.read_rgba(&frame).unwrap()
}

/// The user space of an SVG of natural size `size` in a placed box: the
/// image quad's mapping (`fit` window, anchor at the box center).
fn user_to_device(size: (f64, f64), place: &Place) -> (Affine, [f64; 4]) {
    let uv = pm_render::plan::fit_uv(place.fit, size.0, size.1, place.bw, place.bh);
    let [u0, v0, us, vs] = uv;
    let to_layer = Affine::translate((
        -place.bw / 2.0 - u0 * place.bw / us,
        -place.bh / 2.0 - v0 * place.bh / vs,
    )) * Affine::scale_non_uniform(place.bw / (us * size.0), place.bh / (vs * size.1));
    (place.world() * to_layer, uv)
}

/// The reference frame (premultiplied, 0…1). SVG's painting model: each
/// shape's exact coverage composited per pixel over the ones beneath, then
/// the image's crop (its box and viewport window) as a coverage mask over
/// the composited image.
fn reference(case: &Case, place: &Place) -> Vec<f64> {
    let (m, [u0, v0, us, vs]) = user_to_device(case.size, place);
    let (w, h) = (W as usize, H as usize);
    let (sw, sh) = case.size;
    let corners = [
        ((u0 * sw).max(0.0), (v0 * sh).max(0.0)),
        (((u0 + us) * sw).min(sw), (v0 * sh).max(0.0)),
        (((u0 + us) * sw).min(sw), ((v0 + vs) * sh).min(sh)),
        ((u0 * sw).max(0.0), ((v0 + vs) * sh).min(sh)),
    ]
    .map(|(x, y)| m * Point::new(x, y));
    let mut window = BezPath::new();
    window.move_to(corners[0]);
    for p in &corners[1..] {
        window.line_to(*p);
    }
    window.close_path();
    let shows = fill(&window, FillRule::NonZero, 16, w, h);
    let mut out = vec![0.0; w * h * 4];
    match case.plain {
        Some(paint) => {
            for (color, outline, rule) in paint() {
                let cov = fill(&(m * outline), rule, case.cells, w, h);
                for y in 0..h {
                    for x in 0..w {
                        let c = cov.at(x, y);
                        let px = &mut out[(y * w + x) * 4..][..4];
                        let keep = 1.0 - c * color[3];
                        for i in 0..4 {
                            px[i] = color[i] * c + px[i] * keep;
                        }
                    }
                }
            }
        }
        None => {
            // resvg at 16 × 16 supersampling, box-filtered.
            let dir = Dir::new(&format!("ref-{}", case.name));
            let path = dir.0.join("doc.svg");
            std::fs::write(&path, case.svg).unwrap();
            let (doc, _) = document(&path).unwrap();
            assert_eq!(doc.size(), case.size, "the asset record is the SVG's size");
            let big = doc.raster_device(Affine::scale(16.0) * m, PixelRect::new(0, 0, W * 16, H * 16), None);
            for y in 0..h {
                for x in 0..w {
                    let mut sum = [0.0f64; 4];
                    for sy in 0..16 {
                        let row = (y * 16 + sy) * w * 16;
                        for sx in 0..16 {
                            let i = (row + x * 16 + sx) * 4;
                            for k in 0..4 {
                                sum[k] += f64::from(big.rgba[i + k]);
                            }
                        }
                    }
                    for k in 0..4 {
                        out[(y * w + x) * 4 + k] = sum[k] / (256.0 * 255.0);
                    }
                }
            }
        }
    }
    for (i, px) in out.chunks_mut(4).enumerate() {
        let c = shows.at(i % w, i / w);
        px.iter_mut().for_each(|v| *v *= c);
    }
    out
}

/// `svgRasterDimensions` (legacy/gl/compositor.ts): today's bucket.
fn bucket(sw: f64, sh: f64, bw: f64, bh: f64, m: Affine) -> (u32, u32) {
    let [a, b, c, d, _, _] = m.as_coeffs();
    let (aa, bb, cc) = (a * a + b * b, a * c + b * d, c * c + d * d);
    let largest = ((aa + cc + ((aa - cc).powi(2) + 4.0 * bb * bb).max(0.0).sqrt()) / 2.0).max(0.0).sqrt();
    let density = (bw / sw).max(bh / sh);
    let max_scale = (8192.0 / sw.max(sh)).max(0.25);
    let requested = (largest * density.max(0.01)).min(max_scale.max(0.25)).max(0.25);
    let scale = (requested * 4.0 - 1e-9).ceil() / 4.0;
    ((sw * scale).ceil().max(1.0) as u32, (sh * scale).ceil().max(1.0) as u32)
}

/// Today's frame: the bucket raster through the image quad (bilinear,
/// clamped edges, the quad single-sampled), and where it blends texels that
/// differ (resampled pixels, judged against the reference).
fn today(case: &Case, place: &Place, dir: &Dir) -> (Vec<f64>, Vec<bool>) {
    let path = dir.0.join(format!("today-{}.svg", case.name));
    std::fs::write(&path, case.svg).unwrap();
    let (doc, _) = document(&path).unwrap();
    let (sw, sh) = case.size;
    let (tw, th) = bucket(sw, sh, place.bw, place.bh, place.world());
    let raster = doc.raster(tw, th);
    let [u0, v0, us, vs] = pm_render::plan::fit_uv(place.fit, sw, sh, place.bw, place.bh);
    let inv = place.world().inverse();
    let texel = |x: i64, y: i64| -> [f64; 4] {
        let (x, y) = (x.clamp(0, i64::from(tw) - 1) as usize, y.clamp(0, i64::from(th) - 1) as usize);
        let i = (y * tw as usize + x) * 4;
        std::array::from_fn(|k| f64::from(raster.rgba[i + k]) / 255.0)
    };
    let (w, h) = (W as usize, H as usize);
    let mut out = vec![0.0; w * h * 4];
    let mut blended = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let l = inv * Point::new(x as f64 + 0.5, y as f64 + 0.5);
            let (bx, by) = ((l.x + place.bw / 2.0) / place.bw, (l.y + place.bh / 2.0) / place.bh);
            if !(0.0..1.0).contains(&bx) || !(0.0..1.0).contains(&by) {
                continue;
            }
            let (u, v) = (u0 + bx * us, v0 + by * vs);
            if !(0.0..=1.0).contains(&u) || !(0.0..=1.0).contains(&v) {
                continue;
            }
            let (fx, fy) = (u * f64::from(tw) - 0.5, v * f64::from(th) - 0.5);
            let (ix, iy) = (fx.floor(), fy.floor());
            let (tx, ty) = (fx - ix, fy - iy);
            let (ix, iy) = (ix as i64, iy as i64);
            let (a, b, c, d) = (texel(ix, iy), texel(ix + 1, iy), texel(ix, iy + 1), texel(ix + 1, iy + 1));
            blended[y * w + x] = [b, c, d].iter().any(|t| t.iter().zip(&a).any(|(p, q)| (p - q).abs() > 0.5 / 255.0));
            for k in 0..4 {
                let top = a[k] + (b[k] - a[k]) * tx;
                let bottom = c[k] + (d[k] - c[k]) * tx;
                out[(y * w + x) * 4 + k] = top + (bottom - top) * ty;
            }
        }
    }
    (out, blended)
}

/// Pixels where the reference's paint varies (gradients, masks): today
/// samples its bucket there, so they are judged as resampled pixels (at
/// least as close to the reference as today). Where the paint is flat, a
/// bucket resamples to the authored colour itself: interiors.
fn varying(reference: &[f64]) -> Vec<bool> {
    let (w, h) = (W as usize, H as usize);
    let px = |x: usize, y: usize| &reference[(y * w + x) * 4..][..4];
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            (x.saturating_sub(1)..(x + 2).min(w)).any(|nx| {
                (y.saturating_sub(1)..(y + 2).min(h))
                    .any(|ny| px(x, y).iter().zip(px(nx, ny)).any(|(p, q)| (p - q).abs() > 0.5 / 255.0))
            })
        })
        .collect()
}

/// Edge pixels (grown by 2 px): partly covered in the reference, or where
/// its colour steps.
fn edges(reference: &[f64]) -> Vec<bool> {
    let e = edge_pixels(reference);
    pm_reference::compare::dilate(&e, W as usize, H as usize, 2)
}

fn edge_pixels(reference: &[f64]) -> Vec<bool> {
    let (w, h) = (W as usize, H as usize);
    let px = |x: usize, y: usize| &reference[(y * w + x) * 4..][..4];
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let a = px(x, y)[3];
            if a > 1e-6 && a < 1.0 - 1e-6 {
                return true;
            }
            [(1i64, 0i64), (0, 1), (-1, 0), (0, -1)].iter().any(|&(dx, dy)| {
                let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                nx >= 0
                    && ny >= 0
                    && (nx as usize) < w
                    && (ny as usize) < h
                    && px(x, y)
                        .iter()
                        .zip(px(nx as usize, ny as usize))
                        .any(|(p, q)| (p - q).abs() > 2.0 / 255.0)
            })
        })
        .collect()
}

/// The geometry bound: 1e-3 (`same_geometry`), plus what 8-bit storage of
/// the image allows. Vello writes 8-bit colour, as today's canvas does, so
/// each pixel whose reference alpha is fractional (an edge, or translucent
/// paint such as `fill-opacity="0.75"`, stored as 191/255) may be off by
/// half a code: the area by at most that per such pixel, the centroid by
/// that times its distance from the centroid.
fn geometry_tolerance(reference: &[f64]) -> (f64, f64) {
    let q = 0.5 / 255.0;
    let g = geometry(reference, W as usize);
    let (mut n, mut moment) = (0.0, 0.0f64);
    for (i, a) in reference.iter().enumerate() {
        if *a > 1e-9 && *a < 1.0 - 1e-9 {
            let (x, y) = ((i % W as usize) as f64 + 0.5, (i / W as usize) as f64 + 0.5);
            n += 1.0;
            moment += (x - g.centroid[0]).abs().max((y - g.centroid[1]).abs());
        }
    }
    (1e-3 + q * n / g.area.max(1.0), 1e-3 + q * moment / g.area.max(1.0))
}

#[derive(serde::Serialize)]
struct Measured {
    case: &'static str,
    fit: &'static str,
    scale: f64,
    rotation: f64,
    edge_px: usize,
    engine: [f64; 3],
    today: [f64; 3],
    interior_vs_today: f64,
}

/// One case at one placement: the two-part rule, geometry, preview ==
/// export. Returns the edge numbers (max, p99.9, mean) for engine and today.
fn judge(renderer: &mut Renderer, case: &Case, place: Place, failures: &mut Vec<String>) -> Measured {
    let dir = Dir::new(case.name);
    let (doc, resolver) = project_for(case, &place, &dir);
    let preview = engine(renderer, &doc, &resolver, false);
    let export = engine(renderer, &doc, &resolver, true);
    let at = format!("{} {} {}% {}°", case.name, place.fit, place.scale, place.rotation);
    if let Err(e) = identical(&preview, &export) {
        failures.push(format!("{at}: {e}"));
    }
    let e: Vec<f64> = preview.iter().map(|&v| f64::from(v)).collect();
    let r = reference(case, &place);
    let (t, blended) = today(case, &place, &dir);
    let resampled: Vec<bool> = varying(&r).iter().zip(&blended).map(|(a, b)| *a || *b).collect();
    let classes = classify(&edges(&r), &resampled, W as usize, H as usize, 0);
    let report = compare(&e, &t, &r, &classes);
    for f in report.failures(&SVG_BOUNDS) {
        failures.push(format!("{at}: {f}"));
    }
    let alpha = |f: &[f64]| f.chunks(4).map(|p| p[3]).collect::<Vec<_>>();
    let (ea, ra) = (alpha(&e), alpha(&r));
    let (tolerance_area, tolerance_centroid) = geometry_tolerance(&ra);
    let (ge, gr) = (geometry(&ea, W as usize), geometry(&ra, W as usize));
    let area = (ge.area - gr.area).abs() / gr.area.max(1.0);
    let centroid = (ge.centroid[0] - gr.centroid[0]).abs().max((ge.centroid[1] - gr.centroid[1]).abs());
    if area > tolerance_area || centroid > tolerance_centroid {
        failures.push(format!(
            "{at}: geometry: area {area:.2e} relative (bound {tolerance_area:.2e}), centroid {centroid:.2e} px (bound {tolerance_centroid:.2e})"
        ));
    }
    if let Some(dir) = std::env::var_os("PM_SVG_DIFFS") {
        let png = |name: &str, f: &[f64]| {
            let bytes: Vec<u8> = f.iter().map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8).collect();
            let stem = format!("{}-{}-{}-{}", case.name, place.fit, place.scale, place.rotation);
            support::write_png(&std::path::Path::new(&dir).join(format!("{stem}-{name}.png")), W, H, &bytes);
        };
        png("engine", &e);
        png("today", &t);
        png("reference", &r);
    }
    // Edges and varying paint (both judged against the reference).
    let edge = report.class(Class::Resampled);
    eprintln!("{at}:{report}");
    let s = |x: &pm_reference::compare::Stats| [x.max, x.p999, x.mean];
    Measured {
        case: case.name,
        fit: place.fit,
        scale: place.scale,
        rotation: place.rotation,
        edge_px: edge.engine_vs_reference.pixels,
        engine: s(&edge.engine_vs_reference),
        today: s(&edge.today_vs_reference),
        interior_vs_today: report.class(Class::Interior).engine_vs_today.max,
    }
}

fn evidence(name: &str, rows: &[Measured]) {
    if let Some(dir) = std::env::var_os("PM_SVG_EVIDENCE") {
        let path = std::path::Path::new(&dir).join(name);
        std::fs::write(&path, serde_json::to_string_pretty(rows).unwrap()).unwrap();
    }
}

/// The two-part rule for every document at today's common placements:
/// `contain`, `cover` and `stretch` boxes, rotated and not.
#[test]
fn svg_images_meet_the_reference_and_todays_interiors() {
    let mut renderer = Renderer::headless().unwrap();
    let mut failures = Vec::new();
    let mut rows = Vec::new();
    for case in cases() {
        let (sw, sh) = case.size;
        for place in [
            Place { fit: "contain", bw: sw * 1.2, bh: sh, scale: 100.0, rotation: 0.0 },
            Place { fit: "stretch", bw: sw * 1.5, bh: sh * 1.25, scale: 120.0, rotation: 23.0 },
            Place { fit: "cover", bw: sw * 0.8, bh: sh, scale: 150.0, rotation: -9.0 },
        ] {
            rows.push(judge(&mut renderer, &case, place, &mut failures));
        }
    }
    evidence("gate.json", &rows);
    let unexpected = settle(failures, PENDING);
    assert!(unexpected.is_empty(), "{}", unexpected.join("\n"));
}

/// SVG sharpness under scale: one placement at 25% … 800%, and the frames
/// of an animated scale (each frame's own scale): the engine draws every
/// scale at output resolution, today resamples its bucket.
#[test]
fn svg_sharpness_under_scale() {
    let mut renderer = Renderer::headless().unwrap();
    let mut failures = Vec::new();
    let mut rows = Vec::new();
    let animated: Vec<f64> = (0..8).map(|i| 60.0 + 40.0 * f64::from(i) * 1.37).collect();
    for case in cases().into_iter().filter(|c| matches!(c.name, "shapes" | "art")) {
        let (sw, sh) = case.size;
        for scale in [25.0, 50.0, 100.0, 237.0, 400.0, 800.0].into_iter().chain(animated.iter().copied()) {
            // Above 150% the frame shows the middle of the image.
            let place = Place { fit: "contain", bw: sw * 1.2, bh: sh * 1.2, scale, rotation: 7.0 };
            rows.push(judge(&mut renderer, &case, place, &mut failures));
        }
    }
    eprintln!("\nSVG sharpness under scale (edge error vs the reference, CV: max / p99.9 / mean)");
    for r in &rows {
        eprintln!(
            "  {:<7} {:>6.1}%  engine {:>6.2} {:>6.2} {:>6.3}  today {:>6.2} {:>6.2} {:>6.3}  ({} px)",
            r.case, r.scale, r.engine[0], r.engine[1], r.engine[2], r.today[0], r.today[1], r.today[2], r.edge_px
        );
    }
    evidence("sharpness.json", &rows);
    let unexpected = settle(failures, PENDING_SCALED);
    assert!(unexpected.is_empty(), "{}", unexpected.join("\n"));
}

/// A frame's SVG images are drawn together into atlas pages, always as the
/// frame's own batch: a viewer whose cache holds them from other frames
/// (other batches, other layouts) draws the frame exactly as a cold export.
#[test]
fn svg_frames_draw_the_same_whatever_the_cache_holds() {
    let dir = Dir::new("batch");
    let mut layers = Vec::new();
    let mut resolver = MapResolver::default();
    let mut doc: Value = project(W, H, 2.0, no_fill(), vec![]);
    for (i, case) in cases().iter().enumerate() {
        let path = dir.0.join(format!("{}.svg", case.name));
        std::fs::write(&path, case.svg).unwrap();
        let id = format!("a{i}");
        asset(&mut doc, &id, "image", "svg", case.size.0 as u32, case.size.1 as u32, 0.0);
        resolver.paths.insert(id.clone(), path);
        let mut l = layer(
            &format!("l{i}"),
            "image",
            json!({ "asset": id, "fit": "contain", "w": 40, "h": 30 }),
            24.0 + 36.0 * i as f64,
            36.0 + 70.0 * (i % 2) as f64,
            2.0,
        );
        set(&mut l, "rotation", json!(5.0 * i as f64));
        layers.push(l);
    }
    let with = |subset: &[usize]| {
        let mut d = doc.clone();
        d["layers"] = json!(subset.iter().map(|&i| layers[i].clone()).collect::<Vec<_>>());
        load(&d)
    };
    let all: Vec<usize> = (0..layers.len()).collect();
    let cold = engine(&mut Renderer::headless().unwrap(), &with(&all), &resolver, true);
    let mut viewer = Renderer::headless().unwrap();
    for subset in [&[0usize, 1][..], &[2, 3, 4], &[4, 0, 2]] {
        engine(&mut viewer, &with(subset), &resolver, false);
    }
    identical(&engine(&mut viewer, &with(&all), &resolver, false), &cold).unwrap();
    identical(&engine(&mut viewer, &with(&all), &resolver, true), &cold).unwrap();
}

/// The V-A2 triage's decisions, per document class.
#[test]
fn svg_semantics_follow_the_triage() {
    let dir = Dir::new("semantics");
    let doc = |name: &str, svg: &str| {
        let p = dir.0.join(name);
        std::fs::write(&p, svg).unwrap();
        document(&p).unwrap().0
    };
    let alpha = |r: &pm_vector::render::svg::SvgRaster, x: u32, y: u32| r.rgba[((y * r.width + x) * 4 + 3) as usize];
    // A. No viewBox and no size: the <img> default 300 × 150, content 1:1.
    let no_size = doc("unsized.svg", r##"<svg xmlns="http://www.w3.org/2000/svg"><rect width="100" height="60" fill="#1B2A4A"/></svg>"##);
    assert_eq!(no_size.size(), (300.0, 150.0));
    let r = no_size.raster(300, 150);
    assert_eq!((alpha(&r, 50, 30), alpha(&r, 150, 30), alpha(&r, 50, 100)), (255, 0, 0));
    let wide = doc("wide.svg", r##"<svg xmlns="http://www.w3.org/2000/svg" width="100"><rect width="100" height="60"/></svg>"##);
    assert_eq!(wide.size(), (100.0, 150.0));
    // B. `rx="auto" ry="5"`: corners of radius 5 (SVG 2 §10.2; Blink draws
    // them square).
    let auto = doc("auto.svg", r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><rect x="5" y="5" width="30" height="20" rx="auto" ry="5"/></svg>"##);
    let r = auto.raster(40, 30);
    assert_eq!(alpha(&r, 5, 5), 0, "rounded corner");
    assert_eq!(alpha(&r, 20, 5), 255);
    // Gradients interpolate straight (unpremultiplied) colour, as Skia and
    // Chromium draw SVG gradients: half way to a transparent orange stop,
    // the colour is half way to orange.
    let grad = doc("grad.svg", r##"<svg xmlns="http://www.w3.org/2000/svg" width="101" height="4"><linearGradient id="g"><stop offset="0" stop-color="#FFFFFF"/><stop offset="1" stop-color="#FF0000" stop-opacity="0"/></linearGradient><rect width="101" height="4" fill="url(#g)"/></svg>"##);
    let mid = grad.raster(101, 4);
    let p = &mid.rgba[(2 * 101 + 50) * 4..][..4];
    let (a, g) = (f64::from(p[3]) / 255.0, f64::from(p[1]) / 255.0 / (f64::from(p[3]) / 255.0));
    assert!((a - 0.5).abs() < 0.02 && (g - 0.5).abs() < 0.03, "{p:?}");
    // C′. Chrome's initial font: Times 16 px. `H` is a cap height of
    // Times (0.662 em = 10.6 px) tall, not 12 px Times New Roman's 8.
    let text = doc("text.svg", r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><text x="4" y="22">H</text></svg>"##);
    let r = text.raster(40, 30);
    let rows: Vec<u32> = (0..30).filter(|&y| (0..40).any(|x| alpha(&r, x, y) > 128)).collect();
    let height = rows.last().unwrap() - rows.first().unwrap() + 1;
    assert!((10..=12).contains(&height), "the H is {height} px tall");
    // Unreadable but not broken (no root): a named failure, not a draw.
    let bad = dir.0.join("bad.svg");
    std::fs::write(&bad, b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect").unwrap();
    assert!(document(&bad).is_err());
}

/// Every probe document against Chromium's own raster of it (when the
/// `svg-gate-*` capture is present): interiors agree within 2 CV except
/// where the triage decided otherwise (A, B, C above; blur precision).
#[test]
fn svg_probe_documents_against_chromium() {
    let Some(capture) = support::newest_capture("svg-gate-") else {
        eprintln!("note: no svg-gate-* capture; the probe documents were not compared");
        return;
    };
    let fixture: Value = serde_json::from_str(include_str!("fixtures/svg-gate-docs.json")).unwrap();
    // Decided otherwise (V-A2): intrinsic size (A) and rx="auto" (B).
    let decided = ["gate-size-no-size", "gate-size-w-only-no-vb", "gate-shape-rect-radii"];
    let dir = Dir::new("probes");
    let mut failures = Vec::new();
    let mut compared = 0;
    for d in fixture["docs"].as_array().unwrap() {
        let name = d["name"].as_str().unwrap();
        let path = dir.0.join(format!("{name}.svg"));
        std::fs::write(&path, d["svg"].as_str().unwrap()).unwrap();
        let Ok((doc, _)) = document(&path) else {
            failures.push(format!("{name}: does not parse"));
            continue;
        };
        if decided.contains(&name) {
            continue;
        }
        for size in d["sizes"].as_array().unwrap() {
            let (w, h) = (size[0].as_u64().unwrap() as u32, size[1].as_u64().unwrap() as u32);
            let png = capture.join(format!("{name}-{w}x{h}.png"));
            if !png.exists() {
                continue;
            }
            let (cw, ch, chromium) = support::read_png(&png);
            assert_eq!((cw, ch), (w, h));
            let ours = doc.raster(w, h);
            // Interior: 2 px from any step in either image, and from the border.
            let step = |img: &[u8], x: usize, y: usize| {
                let p = |x: usize, y: usize| &img[(y * w as usize + x) * 4..][..4];
                (x.saturating_sub(2)..(x + 3).min(w as usize)).any(|nx| {
                    (y.saturating_sub(2)..(y + 3).min(h as usize))
                        .any(|ny| p(x, y).iter().zip(p(nx, ny)).any(|(a, b)| a.abs_diff(*b) > 10))
                })
            };
            let (mut interior, mut off) = (0usize, 0usize);
            for y in 2..h as usize - 2 {
                for x in 2..w as usize - 2 {
                    if step(&chromium, x, y) || step(&ours.rgba, x, y) {
                        continue;
                    }
                    interior += 1;
                    let i = (y * w as usize + x) * 4;
                    if (0..4).any(|k| chromium[i + k].abs_diff(ours.rgba[i + k]) > 2) {
                        off += 1;
                    }
                }
            }
            compared += 1;
            if off > 8.max(interior / 1000) {
                failures.push(format!("{name} {w}x{h}: {off} of {interior} interior pixels differ by more than 2 CV"));
            }
        }
    }
    eprintln!("compared {compared} probe rasters with Chromium's");
    assert!(compared > 300, "{compared}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
