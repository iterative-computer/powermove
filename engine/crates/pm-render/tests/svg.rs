//! G-V3, the SVG image gate (docs/engine/gate-policy.md; docs/engine/vector.md,
//! "SVG images"). Every SVG image is drawn at output resolution; the gate
//! proves it against a reference and against today's look:
//!
//! - **Reference.** Authored fills, gradients, clips, masks, opacity and
//!   dashed strokes are evaluated in f64 with pm-reference coverage and
//!   stroke. Text uses usvg's raw shaped glyph paths, before the engine's
//!   contour resolver, with exact pm-reference coverage. No paint is
//!   quantized through an RGBA8 intermediate.
//! - **Today.** Actual legacy-app premultiplied RGBA8 frames, required from
//!   an external capture manifest. Every requested placement, SVG source
//!   hash, frame dimension, encoding, and byte count is checked.
//! - **Rule.** Interiors (2 px from any edge) within 1 CV of today; edges
//!   at least as close to the reference as today (max, p99.9, mean, and per
//!   pixel within 1 CV for 99.9% and 4 CV for any: `SVG_BOUNDS` with the
//!   RGBA8 capture bound); geometry (alpha area and centroid) within 1e-3
//!   of the reference; preview equals export. Engine frames are RGBA32F.
//! - **Sharpness under scale** is the same comparison over 25% … 800% and
//!   an animated scale, printed (and written to `PM_SVG_EVIDENCE/gate.json`
//!   when set).
//! - **Semantics** (V-A2): the triage's decisions per document class, and
//!   every probe document's interior colours against the required Chromium
//!   `svg-gate-*` capture (as `corpus.rs` finds captures).

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_reference::compare::{Bounds, Class, classify, compare, geometry, identical};
use pm_reference::composite::{Operator, porter_duff};
use pm_reference::coverage::{FillRule, fill};
use pm_reference::stroke::outline as reference_stroke;
use pm_render::vector::svg::document;
use pm_render::{MapResolver, RenderRequest, Renderer};
use pm_vector::kurbo::{
    Affine, BezPath, Cap, Circle, Join, Point, Rect, RoundedRect, Shape, Stroke,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use support::doc::*;
use support::ticks;

const W: u32 = 192;
const H: u32 = 144;

/// The comparison includes an RGBA8 Chromium capture, so its storage
/// uncertainty is half a code value. The engine frame stays RGBA32F.
const SVG_BOUNDS: Bounds = Bounds::EIGHT_BIT;

/// A temporary directory, removed when dropped.
struct Dir(std::path::PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let d =
            std::env::temp_dir().join(format!("pm-render-svg-{tag}-{}-{n}", std::process::id()));
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
    /// units). Complex documents have an authored f64 paint reference.
    plain: Option<fn() -> Vec<PlainPaint>>,
    /// Cells per pixel of the exact coverage: stroke outlines overlap at
    /// their joins, where `coverage::fill` is off by up to 1/n per boundary
    /// segment, so they take 64 (4 CV per segment).
    cells: usize,
}

type PlainPaint = ([f64; 4], BezPath, FillRule);

const SHAPES: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="48" viewBox="0 0 80 48"><rect x="4.5" y="6" width="30" height="20.25" fill="#3FCF8E"/><circle cx="56" cy="24" r="17.5" fill="#4C8DFF"/><rect x="10" y="30" width="60" height="14" rx="6" fill="#FFD66B" fill-opacity="0.75"/></svg>"##;

fn shapes() -> Vec<PlainPaint> {
    let rgb = |hex: u32, a: f64| {
        let c = |s: u32| f64::from((hex >> s) & 0xff) / 255.0 * a;
        [c(16), c(8), c(0), a]
    };
    vec![
        (
            rgb(0x3FCF8E, 1.0),
            Rect::new(4.5, 6.0, 34.5, 26.25).to_path(1e-4),
            FillRule::NonZero,
        ),
        (
            rgb(0x4C8DFF, 1.0),
            Circle::new((56.0, 24.0), 17.5).to_path(1e-4),
            FillRule::NonZero,
        ),
        (
            rgb(0xFFD66B, 0.75),
            RoundedRect::new(10.0, 30.0, 70.0, 44.0, 6.0).to_path(1e-4),
            FillRule::NonZero,
        ),
    ]
}

const STROKES: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="40"><path d="M6 34 L20 6 L34 30 L46 8 L58 34" fill="none" stroke="#E8E2CF" stroke-width="3.5" stroke-linejoin="round" stroke-linecap="round"/><rect x="8" y="8" width="16" height="10" fill="none" stroke="#FF6B1A" stroke-width="2"/></svg>"##;

fn strokes() -> Vec<PlainPaint> {
    let mut zig = BezPath::new();
    zig.move_to((6.0, 34.0));
    for p in [(20.0, 6.0), (34.0, 30.0), (46.0, 8.0), (58.0, 34.0)] {
        zig.line_to(p);
    }
    let style = |w: f64, join: Join, cap: Cap| Stroke::new(w).with_join(join).with_caps(cap);
    let outline = |p: &BezPath, s: &Stroke| reference_stroke(p, s, 1e-5).unwrap().path;
    let rect = Rect::new(8.0, 8.0, 24.0, 18.0).to_path(1e-4);
    vec![
        (
            [
                0xE8 as f64 / 255.0,
                0xE2 as f64 / 255.0,
                0xCF as f64 / 255.0,
                1.0,
            ],
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

/// ART's authored geometry and SVG paints, evaluated in f64. The paths are
/// deliberately built from the SVG numbers above, independently of usvg's
/// converted draw tree and the GPU painter.
fn art_reference(m: Affine, window: &BezPath) -> Vec<f64> {
    // Paint and coverage are integrated together in f64 on an 8 × 8 grid.
    // Each grid cell has exact 2 × 2 coverage (16 × 16 per output pixel).
    const S: usize = 8;
    let (w, h) = (W as usize * S, H as usize * S);
    let view = Affine::scale(S as f64) * m * Affine::scale(0.3); // viewBox → natural size
    let inverse = view.inverse();
    let mut out = vec![[0.0; 4]; w * h];
    let color = |hex: u32| -> [f64; 3] {
        std::array::from_fn(|i| f64::from((hex >> (16 - 8 * i)) & 255) / 255.0)
    };
    let interpolate =
        |a: [f64; 3], b: [f64; 3], t: f64| std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t);
    let mut paint = |path: BezPath, cells: usize, shade: &dyn Fn(f64, f64) -> ([f64; 3], f64)| {
        let cov = fill(&(view * path), FillRule::NonZero, cells, w, h);
        for y in cov.y0..cov.y0 + cov.height {
            for x in cov.x0..cov.x0 + cov.width {
                let coverage = cov.at(x, y);
                if coverage == 0.0 {
                    continue;
                }
                let p = inverse * Point::new(x as f64 + 0.5, y as f64 + 0.5);
                let (rgb, opacity) = shade(p.x, p.y);
                let alpha = coverage * opacity;
                let src = [rgb[0] * alpha, rgb[1] * alpha, rgb[2] * alpha, alpha];
                let px = &mut out[y * w + x];
                *px = porter_duff(Operator::SourceOver, src, *px);
            }
        }
    };
    paint(
        Rect::new(0.0, 0.0, 320.0, 180.0).to_path(1e-6),
        2,
        &|x, y| {
            // SVG objectBoundingBox gradients are projected in normalized box
            // coordinates, before the nonuniform box transform is applied.
            let t = ((x / 320.0 + y / 180.0) * 0.5).clamp(0.0, 1.0);
            let rgb = if t < 0.6 {
                interpolate(color(0x1B2A4A), color(0x4C8DFF), t / 0.6)
            } else {
                interpolate(color(0x4C8DFF), color(0xFFD66B), (t - 0.6) / 0.4)
            };
            (rgb, 1.0)
        },
    );
    paint(
        Circle::new((230.0, 70.0), 46.0).to_path(1e-6),
        2,
        &|x, y| {
            let t = ((x - 230.0).hypot(y - 70.0) / 46.0).clamp(0.0, 1.0);
            (interpolate(color(0xFFF4D0), color(0xFF6B1A), t), 1.0 - t)
        },
    );
    let mut wave = BezPath::new();
    wave.move_to((0.0, 150.0));
    wave.curve_to((60.0, 110.0), (110.0, 170.0), (170.0, 130.0));
    wave.curve_to((230.0, 90.0), (270.0, 100.0), (320.0, 140.0));
    wave.line_to((320.0, 180.0));
    wave.line_to((0.0, 180.0));
    wave.close_path();
    paint(wave, 2, &|_, _| (color(0x0C0D12), 0.85));
    // Dash phase starts at the SVG rect path's first point (-24,-18), not
    // kurbo RoundedRect's first point on the left side.
    let mut rectangle = BezPath::new();
    let k = 6.0 * 0.552_284_749_830_793_6;
    rectangle.move_to((-24.0, -18.0));
    rectangle.line_to((24.0, -18.0));
    rectangle.curve_to((24.0 + k, -18.0), (30.0, -12.0 - k), (30.0, -12.0));
    rectangle.line_to((30.0, 12.0));
    rectangle.curve_to((30.0, 12.0 + k), (24.0 + k, 18.0), (24.0, 18.0));
    rectangle.line_to((-24.0, 18.0));
    rectangle.curve_to((-24.0 - k, 18.0), (-30.0, 12.0 + k), (-30.0, 12.0));
    rectangle.line_to((-30.0, -12.0));
    rectangle.curve_to((-30.0, -12.0 - k), (-24.0 - k, -18.0), (-24.0, -18.0));
    rectangle.close_path();
    let dashed_style = Stroke::new(4.0)
        .with_join(Join::Miter)
        .with_caps(Cap::Butt)
        .with_dashes(0.0, [10.0, 6.0]);
    let dashed = reference_stroke(&rectangle, &dashed_style, 1e-5)
        .unwrap()
        .path;
    let moved = Affine::translate((70.0, 60.0)) * Affine::rotate((-12.0f64).to_radians()) * dashed;
    paint(moved, 8, &|_, _| (color(0xE8E2CF), 0.7));
    // The rounded clip applies once to the isolated group, after its paints.
    let clip = RoundedRect::new(8.0, 8.0, 312.0, 172.0, 18.0).to_path(1e-6);
    let coverage = fill(&(view * clip), FillRule::NonZero, 2, w, h);
    let crop = fill(
        &(Affine::scale(S as f64) * window.clone()),
        FillRule::NonZero,
        2,
        w,
        h,
    );
    let mut down = vec![0.0; W as usize * H as usize * 4];
    for (i, px) in out.iter_mut().enumerate() {
        let c = coverage.at(i % w, i / w) * crop.at(i % w, i / w);
        for (k, v) in px.iter_mut().enumerate() {
            *v *= c;
            down[((i / w / S) * W as usize + (i % w / S)) * 4 + k] += *v / (S * S) as f64;
        }
    }
    down
}

/// MASKED's authored radial paint and luminance mask. The grayscale mask
/// colours make its luminance independent of colour-space coefficients.
fn masked_reference(m: Affine, window: &BezPath) -> Vec<f64> {
    const S: usize = 8;
    let (w, h) = (W as usize * S, H as usize * S);
    let view = Affine::scale(S as f64) * m * Affine::scale(0.3); // viewBox → natural size
    let inv = view.inverse();
    let rect = Rect::new(0.0, 0.0, 200.0, 200.0).to_path(1e-6);
    let background = fill(&(view * rect), FillRule::NonZero, 2, w, h);
    let outer = fill(
        &(view * Circle::new((100.0, 100.0), 80.0).to_path(1e-6)),
        FillRule::NonZero,
        2,
        w,
        h,
    );
    let inner = fill(
        &(view * Circle::new((100.0, 100.0), 40.0).to_path(1e-6)),
        FillRule::NonZero,
        2,
        w,
        h,
    );
    let crop = fill(
        &(Affine::scale(S as f64) * window.clone()),
        FillRule::NonZero,
        2,
        w,
        h,
    );
    let mut diagonals = BezPath::new();
    diagonals.move_to((0.0, 0.0));
    diagonals.line_to((200.0, 200.0));
    diagonals.move_to((200.0, 0.0));
    diagonals.line_to((0.0, 200.0));
    let stroke_path = reference_stroke(
        &diagonals,
        &Stroke::new(12.0)
            .with_join(Join::Miter)
            .with_caps(Cap::Butt),
        1e-5,
    )
    .unwrap()
    .path;
    let stroke_cov = fill(&(view * stroke_path), FillRule::NonZero, 8, w, h);
    let a = [
        0x7C as f64 / 255.0,
        0xF3 as f64 / 255.0,
        0xC1 as f64 / 255.0,
    ];
    let b = [
        0x14 as f64 / 255.0,
        0x60 as f64 / 255.0,
        0x5A as f64 / 255.0,
    ];
    let ink = [
        0x0B as f64 / 255.0,
        0x1F as f64 / 255.0,
        0x1D as f64 / 255.0,
    ];
    let mut out = vec![0.0; W as usize * H as usize * 4];
    for y in 0..h {
        for x in 0..w {
            let p = inv * Point::new(x as f64 + 0.5, y as f64 + 0.5);
            let t = ((p.x - 100.0).hypot(p.y - 100.0) / 90.0).clamp(0.0, 1.0);
            let base = background.at(x, y);
            let s = stroke_cov.at(x, y);
            let mask =
                outer.at(x, y) * (1.0 - inner.at(x, y) * (1.0 - 85.0 / 255.0)) * crop.at(x, y);
            let px = &mut out[((y / S) * W as usize + (x / S)) * 4..][..4];
            for k in 0..3 {
                let gradient = a[k] + (b[k] - a[k]) * t;
                px[k] += (gradient * base * (1.0 - s) + ink[k] * s) * mask / (S * S) as f64;
            }
            px[3] += (base + s * (1.0 - base)) * mask / (S * S) as f64;
        }
    }
    out
}

fn text_reference(m: Affine, window: &BezPath) -> Vec<f64> {
    let doc = pm_vector::render::svg::SvgDocument::parse(TEXT.as_bytes()).unwrap();
    let paths = doc.raw_text_fill_paths();
    assert!(
        !paths.is_empty(),
        "the SVG text must shape to glyph outlines"
    );
    let color = [0x4C as f64 / 255.0, 0x8D as f64 / 255.0, 1.0, 1.0];
    let paints = paths
        .into_iter()
        .map(|(path, even_odd)| {
            (
                color,
                path,
                if even_odd {
                    FillRule::EvenOdd
                } else {
                    FillRule::NonZero
                },
            )
        })
        .collect();
    solid_reference(paints, m, window, 32)
}

/// Authored solid paths composited and cropped on a common f64 grid. The
/// viewport edge can cross another painted edge inside a device pixel.
fn solid_reference(
    paints: Vec<([f64; 4], BezPath, FillRule)>,
    m: Affine,
    window: &BezPath,
    cells: usize,
) -> Vec<f64> {
    const S: usize = 8;
    let (w, h) = (W as usize * S, H as usize * S);
    let mut fine = vec![[0.0f64; 4]; w * h];
    for (color, outline, rule) in paints {
        let coverage = fill(
            &(Affine::scale(S as f64) * m * outline),
            rule,
            (cells / S).max(2),
            w,
            h,
        );
        for y in coverage.y0..coverage.y0 + coverage.height {
            for x in coverage.x0..coverage.x0 + coverage.width {
                let c = coverage.at(x, y);
                let px = &mut fine[y * w + x];
                let keep = 1.0 - color[3] * c;
                for k in 0..4 {
                    px[k] = color[k] * c + px[k] * keep;
                }
            }
        }
    }
    let crop = fill(
        &(Affine::scale(S as f64) * window.clone()),
        FillRule::NonZero,
        2,
        w,
        h,
    );
    let mut out = vec![0.0; W as usize * H as usize * 4];
    for (i, pixel) in fine.iter().enumerate() {
        let c = crop.at(i % w, i / w) / (S * S) as f64;
        let at = ((i / w / S) * W as usize + (i % w / S)) * 4;
        for k in 0..4 {
            out[at + k] += pixel[k] * c;
        }
    }
    out
}

fn plain_reference(case: &Case, m: Affine, window: &BezPath) -> Vec<f64> {
    solid_reference(case.plain.unwrap()(), m, window, case.cells)
}

/// Gradients, a rounded clip, path and group opacity, dashes.
const ART: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="96" height="54" viewBox="0 0 320 180"><defs><linearGradient id="a" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#1B2A4A"/><stop offset="0.6" stop-color="#4C8DFF"/><stop offset="1" stop-color="#FFD66B"/></linearGradient><radialGradient id="b" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#FFF4D0"/><stop offset="1" stop-color="#FF6B1A" stop-opacity="0"/></radialGradient><clipPath id="c"><rect x="8" y="8" width="304" height="164" rx="18"/></clipPath></defs><g clip-path="url(#c)"><rect width="320" height="180" fill="url(#a)"/><circle cx="230" cy="70" r="46" fill="url(#b)"/><path d="M0 150 C 60 110, 110 170, 170 130 S 270 100, 320 140 L 320 180 L 0 180 Z" fill="#0C0D12" opacity="0.85"/><g opacity="0.7" transform="translate(70 60) rotate(-12)"><rect x="-30" y="-18" width="60" height="36" rx="6" fill="none" stroke="#E8E2CF" stroke-width="4" stroke-dasharray="10 6"/></g></g></svg>"##;

/// A luminance mask over a radial gradient, crossed strokes.
const MASKED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="60" height="60" viewBox="0 0 200 200"><defs><radialGradient id="g" cx="100" cy="100" r="90" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#7CF3C1"/><stop offset="1" stop-color="#14605A"/></radialGradient><mask id="m"><rect width="200" height="200" fill="#000"/><circle cx="100" cy="100" r="80" fill="#fff"/><circle cx="100" cy="100" r="40" fill="#555"/></mask></defs><g mask="url(#m)"><rect width="200" height="200" fill="url(#g)"/><path d="M0 0 L200 200 M200 0 L0 200" stroke="#0B1F1D" stroke-width="12"/></g></svg>"##;

const TEXT: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="30"><text x="4" y="22" font-family="Helvetica" font-size="18" fill="#4C8DFF">Pm Svg</text></svg>"##;

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "shapes",
            svg: SHAPES,
            size: (80.0, 48.0),
            plain: Some(shapes),
            cells: 16,
        },
        Case {
            name: "strokes",
            svg: STROKES,
            size: (64.0, 40.0),
            plain: Some(strokes),
            cells: 64,
        },
        Case {
            name: "art",
            svg: ART,
            size: (96.0, 54.0),
            plain: None,
            cells: 16,
        },
        Case {
            name: "masked",
            svg: MASKED,
            size: (60.0, 60.0),
            plain: None,
            cells: 16,
        },
        Case {
            name: "text",
            svg: TEXT,
            size: (80.0, 30.0),
            plain: None,
            cells: 16,
        },
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
    doc["colorDepth"] = json!(32);
    asset(
        &mut doc,
        "art",
        "image",
        "svg",
        case.size.0 as u32,
        case.size.1 as u32,
        0.0,
    );
    let mut resolver = MapResolver::default();
    resolver.paths.insert("art".into(), path);
    (load(&doc), resolver)
}

/// The engine's frame (premultiplied, 0…1), as a preview or an export.
fn engine(
    renderer: &mut Renderer,
    doc: &pm_model::Document,
    resolver: &MapResolver,
    export: bool,
) -> Vec<f32> {
    let mut request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    request.export = export;
    let frame = renderer
        .render(doc, "Ptest-root", ticks(0.0), &request, resolver)
        .expect("render");
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    assert_eq!(frame.color.format(), wgpu::TextureFormat::Rgba32Float);
    renderer.read_rgba(&frame).unwrap()
}

/// The user space of an SVG of natural size `size` in a placed box: the
/// image quad's mapping (`fit` window, anchor at the box center).
fn user_to_device(size: (f64, f64), place: &Place) -> (Affine, [f64; 4]) {
    let uv = pm_render::plan::fit_uv(place.fit, size.0, size.1, place.bw, place.bh);
    let [u0, v0, us, vs] = uv;
    let to_layer =
        Affine::translate((
            -place.bw / 2.0 - u0 * place.bw / us,
            -place.bh / 2.0 - v0 * place.bh / vs,
        )) * Affine::scale_non_uniform(place.bw / (us * size.0), place.bh / (vs * size.1));
    (place.world() * to_layer, uv)
}

/// The reference frame (premultiplied, 0…1). Authored paints, group effects,
/// and the viewport crop share a fine f64 grid before pixel averaging.
fn reference(case: &Case, place: &Place) -> Vec<f64> {
    let (m, [u0, v0, us, vs]) = user_to_device(case.size, place);
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
    match case.plain {
        Some(_) => plain_reference(case, m, &window),
        None if case.name == "art" => art_reference(m, &window),
        None if case.name == "masked" => masked_reference(m, &window),
        None if case.name == "text" => text_reference(m, &window),
        None => unreachable!("every complex SVG case has an authored reference"),
    }
}

#[derive(serde::Deserialize)]
struct FrameManifest {
    version: u32,
    width: u32,
    height: u32,
    encoding: String,
    capture: String,
    versions: FrameVersions,
    records: Vec<FrameRecord>,
}

#[derive(serde::Deserialize)]
struct FrameVersions {
    electron: String,
    chromium: String,
}

#[derive(serde::Deserialize)]
struct FrameRecord {
    case: String,
    fit: String,
    scale: f64,
    rotation: f64,
    bw: f64,
    bh: f64,
    source_sha256: String,
    file: String,
    sha256: String,
}

/// Actual legacy-app frame, captured externally as top-down premultiplied
/// RGBA8. Missing or mismatched evidence fails the gate; resvg is never a
/// substitute for today's pixels.
fn today(case: &Case, place: &Place) -> Vec<f64> {
    let location = std::env::var_os("PM_SVG_FRAME_REF")
        .map(std::path::PathBuf::from)
        .or_else(|| support::newest_capture("svg-frames-"))
        .expect("G-V3 requires PM_SVG_FRAME_REF or a svg-frames-* legacy-app capture");
    let manifest_path = if location.is_dir() {
        location.join("manifest.json")
    } else {
        location
    };
    let manifest: FrameManifest = serde_json::from_slice(
        &std::fs::read(&manifest_path)
            .unwrap_or_else(|e| panic!("{}: {e}", manifest_path.display())),
    )
    .unwrap_or_else(|e| panic!("{}: {e}", manifest_path.display()));
    assert_eq!(manifest.version, 1, "{}", manifest_path.display());
    assert_eq!(
        (manifest.width, manifest.height),
        (W, H),
        "{}",
        manifest_path.display()
    );
    assert_eq!(
        manifest.encoding,
        "premultiplied-rgba8-topdown",
        "{}",
        manifest_path.display()
    );
    assert!(manifest.capture.starts_with("legacy PM.GL.renderToPixels"));
    assert!(!manifest.versions.electron.is_empty() && !manifest.versions.chromium.is_empty());
    assert_eq!(manifest.records.len(), 43, "incomplete SVG frame capture");
    let same = |a: f64, b: f64| (a - b).abs() <= 1e-9;
    let matching: Vec<_> = manifest
        .records
        .iter()
        .filter(|row| {
            row.case == case.name
                && row.fit == place.fit
                && same(row.scale, place.scale)
                && same(row.rotation, place.rotation)
                && same(row.bw, place.bw)
                && same(row.bh, place.bh)
        })
        .collect();
    assert_eq!(
        matching.len(),
        1,
        "{} {} {}% {}°: expected exactly one captured legacy frame in {}",
        case.name,
        place.fit,
        place.scale,
        place.rotation,
        manifest_path.display()
    );
    let row = matching[0];
    let source_sha256 = format!("{:x}", Sha256::digest(case.svg.as_bytes()));
    assert_eq!(
        row.source_sha256,
        source_sha256,
        "{}: SVG source hash mismatch",
        manifest_path.display()
    );
    let basename = std::path::Path::new(&row.file);
    assert_eq!(
        basename.file_name().and_then(|s| s.to_str()),
        Some(row.file.as_str()),
        "capture file must be a basename"
    );
    assert_eq!(
        basename.extension().and_then(|s| s.to_str()),
        Some("rgba"),
        "capture must be raw RGBA8"
    );
    let file = manifest_path.parent().unwrap().join(basename);
    let bytes = std::fs::read(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    assert_eq!(
        bytes.len(),
        (W as usize) * (H as usize) * 4,
        "{}: incorrect RGBA8 length",
        file.display()
    );
    assert_eq!(
        format!("{:x}", Sha256::digest(&bytes)),
        row.sha256,
        "{}: frame hash mismatch",
        file.display()
    );
    bytes.into_iter().map(|v| f64::from(v) / 255.0).collect()
}

/// Pixels whose reference or captured legacy frame varies with neighboring
/// pixels are judged as resampled. This classification reads no engine pixel.
fn varying(reference: &[f64]) -> Vec<bool> {
    let (w, h) = (W as usize, H as usize);
    let px = |x: usize, y: usize| &reference[(y * w + x) * 4..][..4];
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            (x.saturating_sub(1)..(x + 2).min(w)).any(|nx| {
                (y.saturating_sub(1)..(y + 2).min(h)).any(|ny| {
                    px(x, y)
                        .iter()
                        .zip(px(nx, ny))
                        .any(|(p, q)| (p - q).abs() > 0.5 / 255.0)
                })
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
            [(1i64, 0i64), (0, 1), (-1, 0), (0, -1)]
                .iter()
                .any(|&(dx, dy)| {
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
    area_relative: f64,
    centroid_px: f64,
}

/// One case at one placement: the two-part rule, geometry, preview ==
/// export. Returns the edge numbers (max, p99.9, mean) for engine and today.
fn judge(
    renderer: &mut Renderer,
    case: &Case,
    place: Place,
    failures: &mut Vec<String>,
) -> Measured {
    let dir = Dir::new(case.name);
    let (doc, resolver) = project_for(case, &place, &dir);
    let preview = engine(renderer, &doc, &resolver, false);
    let export = engine(renderer, &doc, &resolver, true);
    let at = format!(
        "{} {} {}% {}°",
        case.name, place.fit, place.scale, place.rotation
    );
    if let Err(e) = identical(&preview, &export) {
        failures.push(format!("{at}: {e}"));
    }
    let e: Vec<f64> = preview.iter().map(|&v| f64::from(v)).collect();
    let r = reference(case, &place);
    let t = today(case, &place);
    let legacy_neighbors = varying(&t);
    let resampled: Vec<bool> = varying(&r)
        .iter()
        .zip(&legacy_neighbors)
        .map(|(a, b)| *a || *b)
        .collect();
    let classes = classify(&edges(&r), &resampled, W as usize, H as usize, 0);
    let report = compare(&e, &t, &r, &classes);
    for f in report.failures(&SVG_BOUNDS) {
        failures.push(format!("{at}: {f}"));
    }
    let alpha = |f: &[f64]| f.chunks(4).map(|p| p[3]).collect::<Vec<_>>();
    let (ea, ra) = (alpha(&e), alpha(&r));
    let (ge, gr) = (geometry(&ea, W as usize), geometry(&ra, W as usize));
    let area = (ge.area - gr.area).abs() / gr.area.max(1.0);
    let centroid = (ge.centroid[0] - gr.centroid[0])
        .abs()
        .max((ge.centroid[1] - gr.centroid[1]).abs());
    if area > 1e-3 || centroid > 1e-3 {
        failures.push(format!(
            "{at}: geometry: area {area:.2e} relative (bound 1e-3), centroid {centroid:.2e} px (bound 1e-3)"
        ));
    }
    if let Some(dir) = std::env::var_os("PM_SVG_DIFFS") {
        let png = |name: &str, f: &[f64]| {
            let bytes: Vec<u8> = f
                .iter()
                .map(|v| (v * 255.0).round().clamp(0.0, 255.0) as u8)
                .collect();
            let stem = format!(
                "{}-{}-{}-{}",
                case.name, place.fit, place.scale, place.rotation
            );
            support::write_png(
                &std::path::Path::new(&dir).join(format!("{stem}-{name}.png")),
                W,
                H,
                &bytes,
            );
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
        area_relative: area,
        centroid_px: centroid,
    }
}

fn evidence(name: &str, rows: &[Measured]) {
    if let Some(dir) = std::env::var_os("PM_SVG_EVIDENCE") {
        let root = std::path::Path::new(&dir);
        std::fs::create_dir_all(root).unwrap();
        let path = root.join(name);
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
            Place {
                fit: "contain",
                bw: sw * 1.2,
                bh: sh,
                scale: 100.0,
                rotation: 0.0,
            },
            Place {
                fit: "stretch",
                bw: sw * 1.5,
                bh: sh * 1.25,
                scale: 120.0,
                rotation: 23.0,
            },
            Place {
                fit: "cover",
                bw: sw * 0.8,
                bh: sh,
                scale: 150.0,
                rotation: -9.0,
            },
        ] {
            rows.push(judge(&mut renderer, &case, place, &mut failures));
        }
    }
    evidence("gate.json", &rows);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// SVG sharpness under scale: one placement at 25% … 800%, and the frames
/// of an animated scale (each frame's own scale): compare output-resolution
/// engine frames with captured legacy-app frames at the same placement.
#[test]
fn svg_sharpness_under_scale() {
    let mut renderer = Renderer::headless().unwrap();
    let mut failures = Vec::new();
    let mut rows = Vec::new();
    let animated: Vec<f64> = (0..8).map(|i| 60.0 + 40.0 * f64::from(i) * 1.37).collect();
    for case in cases()
        .into_iter()
        .filter(|c| matches!(c.name, "shapes" | "art"))
    {
        let (sw, sh) = case.size;
        for scale in [25.0, 50.0, 100.0, 237.0, 400.0, 800.0]
            .into_iter()
            .chain(animated.iter().copied())
        {
            // Above 150% the frame shows the middle of the image.
            let place = Place {
                fit: "contain",
                bw: sw * 1.2,
                bh: sh * 1.2,
                scale,
                rotation: 7.0,
            };
            rows.push(judge(&mut renderer, &case, place, &mut failures));
        }
    }
    eprintln!("\nSVG sharpness under scale (edge error vs the reference, CV: max / p99.9 / mean)");
    for r in &rows {
        eprintln!(
            "  {:<7} {:>6.1}%  engine {:>6.2} {:>6.2} {:>6.3}  today {:>6.2} {:>6.2} {:>6.3}  ({} px)",
            r.case,
            r.scale,
            r.engine[0],
            r.engine[1],
            r.engine[2],
            r.today[0],
            r.today[1],
            r.today[2],
            r.edge_px
        );
    }
    evidence("sharpness.json", &rows);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn art_wave_and_clip_crossings_match_the_f64_reference() {
    let case = cases().into_iter().find(|c| c.name == "art").unwrap();
    let mut renderer = Renderer::headless().unwrap();
    for (scale, x, y) in [(50.0, 67, 78), (100.0, 148, 94), (60.0, 127, 85)] {
        let place = Place {
            fit: "contain",
            bw: case.size.0 * 1.2,
            bh: case.size.1 * 1.2,
            scale,
            rotation: 7.0,
        };
        let dir = Dir::new("art-crossing");
        let (doc, resolver) = project_for(&case, &place, &dir);
        let actual = engine(&mut renderer, &doc, &resolver, true);
        let expected = reference(&case, &place);
        let offset = ((y * W + x) * 4) as usize;
        for k in 0..4 {
            let error_cv = (f64::from(actual[offset + k]) - expected[offset + k]).abs() * 255.0;
            assert!(
                error_cv < 1.0,
                "scale {scale}, ({x},{y}), channel {k}: {error_cv:.3} CV"
            );
        }
    }
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
    doc["colorDepth"] = json!(32);
    for (i, case) in cases().iter().enumerate() {
        let path = dir.0.join(format!("{}.svg", case.name));
        std::fs::write(&path, case.svg).unwrap();
        let id = format!("a{i}");
        asset(
            &mut doc,
            &id,
            "image",
            "svg",
            case.size.0 as u32,
            case.size.1 as u32,
            0.0,
        );
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
        d["layers"] = json!(
            subset
                .iter()
                .map(|&i| layers[i].clone())
                .collect::<Vec<_>>()
        );
        load(&d)
    };
    let all: Vec<usize> = (0..layers.len()).collect();
    let cold = engine(
        &mut Renderer::headless().unwrap(),
        &with(&all),
        &resolver,
        true,
    );
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
    let alpha = |r: &pm_vector::render::svg::SvgRaster, x: u32, y: u32| {
        r.rgba[((y * r.width + x) * 4 + 3) as usize]
    };
    // A. No viewBox and no size: the <img> default 300 × 150, content 1:1.
    let no_size = doc(
        "unsized.svg",
        r##"<svg xmlns="http://www.w3.org/2000/svg"><rect width="100" height="60" fill="#1B2A4A"/></svg>"##,
    );
    assert_eq!(no_size.size(), (300.0, 150.0));
    let r = no_size.raster(300, 150);
    assert_eq!(
        (alpha(&r, 50, 30), alpha(&r, 150, 30), alpha(&r, 50, 100)),
        (255, 0, 0)
    );
    let wide = doc(
        "wide.svg",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100"><rect width="100" height="60"/></svg>"##,
    );
    assert_eq!(wide.size(), (100.0, 150.0));
    // B. `rx="auto" ry="5"`: corners of radius 5 (SVG 2 §10.2; Blink draws
    // them square).
    let auto = doc(
        "auto.svg",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><rect x="5" y="5" width="30" height="20" rx="auto" ry="5"/></svg>"##,
    );
    let r = auto.raster(40, 30);
    assert_eq!(alpha(&r, 5, 5), 0, "rounded corner");
    assert_eq!(alpha(&r, 20, 5), 255);
    // Gradients interpolate straight (unpremultiplied) colour, as Skia and
    // Chromium draw SVG gradients: half way to a transparent orange stop,
    // the colour is half way to orange.
    let grad = doc(
        "grad.svg",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="101" height="4"><linearGradient id="g"><stop offset="0" stop-color="#FFFFFF"/><stop offset="1" stop-color="#FF0000" stop-opacity="0"/></linearGradient><rect width="101" height="4" fill="url(#g)"/></svg>"##,
    );
    let mid = grad.raster(101, 4);
    let p = &mid.rgba[(2 * 101 + 50) * 4..][..4];
    let (a, g) = (
        f64::from(p[3]) / 255.0,
        f64::from(p[1]) / 255.0 / (f64::from(p[3]) / 255.0),
    );
    assert!((a - 0.5).abs() < 0.02 && (g - 0.5).abs() < 0.03, "{p:?}");
    // C′. Chrome's initial font: Times 16 px. `H` is a cap height of
    // Times (0.662 em = 10.6 px) tall, not 12 px Times New Roman's 8.
    let text = doc(
        "text.svg",
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="30"><text x="4" y="22">H</text></svg>"##,
    );
    let r = text.raster(40, 30);
    let rows: Vec<u32> = (0..30)
        .filter(|&y| (0..40).any(|x| alpha(&r, x, y) > 128))
        .collect();
    let height = rows.last().unwrap() - rows.first().unwrap() + 1;
    assert!((10..=12).contains(&height), "the H is {height} px tall");
    // Unreadable but not broken (no root): a named failure, not a draw.
    let bad = dir.0.join("bad.svg");
    std::fs::write(&bad, b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect").unwrap();
    assert!(document(&bad).is_err());
}

/// Every probe document against Chromium's own raster of it: interiors
/// agree within 2 CV except
/// where the triage decided otherwise (A, B, C above; blur precision).
#[test]
fn svg_probe_documents_against_chromium() {
    let capture = support::newest_capture("svg-gate-")
        .expect("the SVG gate requires a complete svg-gate-* Chromium capture");
    let fixture: Value = serde_json::from_str(include_str!("fixtures/svg-gate-docs.json")).unwrap();
    // Decided otherwise (V-A2): intrinsic size (A) and rx="auto" (B).
    let decided = [
        "gate-size-no-size",
        "gate-size-w-only-no-vb",
        "gate-shape-rect-radii",
    ];
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
            let (w, h) = (
                size[0].as_u64().unwrap() as u32,
                size[1].as_u64().unwrap() as u32,
            );
            let png = capture.join(format!("{name}-{w}x{h}.png"));
            assert!(png.exists(), "missing Chromium witness: {}", png.display());
            let (cw, ch, chromium) = support::read_png(&png);
            assert_eq!((cw, ch), (w, h));
            let ours = doc.raster(w, h);
            // Interior: 2 px from any step in either image, and from the border.
            let step = |img: &[u8], x: usize, y: usize| {
                let p = |x: usize, y: usize| &img[(y * w as usize + x) * 4..][..4];
                (x.saturating_sub(2)..(x + 3).min(w as usize)).any(|nx| {
                    (y.saturating_sub(2)..(y + 3).min(h as usize)).any(|ny| {
                        p(x, y)
                            .iter()
                            .zip(p(nx, ny))
                            .any(|(a, b)| a.abs_diff(*b) > 10)
                    })
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
                failures.push(format!(
                    "{name} {w}x{h}: {off} of {interior} interior pixels differ by more than 2 CV"
                ));
            }
        }
    }
    eprintln!("compared {compared} probe rasters with Chromium's");
    assert!(compared > 300, "{compared}");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
