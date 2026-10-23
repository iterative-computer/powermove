//! Vector layers (plan section 10; docs/engine/vector.md): `compat` level 2
//! shapes at output resolution with analytic edges, Vello on the GPU
//! against the `vello_cpu` fallback, the solid `radius` on level 2 only, and
//! the vector cache's memory accounting.
//!
//! Level 2 references are computed here, not captured: each pixel's exact
//! coverage of the transformed geometry, estimated with 16 × 16 samples.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::vector::cache::CoverageBackend;
use pm_render::{Feature, MapResolver, RenderRequest, Renderer, unsupported_with_media};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

const W: u32 = 256;
const H: u32 = 256;

/// A layer at the composition's center, rotated by `rotation` degrees and
/// scaled by `scale` percent.
fn placed(id: &str, ty: &str, d: Value, compat: u32, rotation: f64, scale: f64) -> Value {
    let mut l = layer(id, ty, d, 128.0, 128.0, 2.0);
    set(&mut l, "rotation", json!(rotation));
    set(&mut l, "scale.x", json!(scale));
    set(&mut l, "scale.y", json!(scale));
    if compat > 1 {
        l["compat"] = json!(compat);
    }
    l
}

fn render_alpha(renderer: &mut Renderer, layers: Vec<Value>) -> Vec<f32> {
    render_rgba(
        renderer,
        project(W, H, 2.0, no_fill(), layers),
        &MapResolver::default(),
    )
    .chunks(4)
    .map(|p| p[3])
    .collect()
}

fn render_rgba(renderer: &mut Renderer, doc: Value, resolver: &MapResolver) -> Vec<f32> {
    let doc = load(&doc);
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    let frame = renderer
        .render(&doc, "Ptest-root", ticks(0.0), &request, resolver)
        .expect("render");
    assert!(frame.failures.is_empty(), "{:?}", frame.failures);
    renderer.read_rgba(&frame).unwrap()
}

/// Exact coverage of each pixel by `inside` (composition pixels), with
/// 16 × 16 samples per pixel.
fn coverage(inside: impl Fn(f64, f64) -> bool) -> Vec<f32> {
    let mut out = vec![0.0f32; (W * H) as usize];
    for y in 0..H {
        for x in 0..W {
            let mut n = 0u32;
            for sy in 0..16 {
                for sx in 0..16 {
                    let px = f64::from(x) + (f64::from(sx) + 0.5) / 16.0;
                    let py = f64::from(y) + (f64::from(sy) + 0.5) / 16.0;
                    n += u32::from(inside(px, py));
                }
            }
            out[(y * W + x) as usize] = n as f32 / 256.0;
        }
    }
    out
}

/// Composition point to layer units for a layer at the center rotated by
/// `rotation` degrees and scaled by `scale` percent.
fn to_layer(px: f64, py: f64, rotation: f64, scale: f64) -> (f64, f64) {
    let (s, c) = (-rotation.to_radians()).sin_cos();
    let (dx, dy) = (px - 128.0, py - 128.0);
    let k = 100.0 / scale;
    ((dx * c - dy * s) * k, (dx * s + dy * c) * k)
}

fn point_in_polygon(poly: &[(f64, f64)], x: f64, y: f64) -> bool {
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let (xi, yi) = poly[i];
        let (xj, yj) = poly[j];
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Error statistics over the reference's edge pixels (partial coverage),
/// and the largest error anywhere.
struct EdgeError {
    edge_pixels: usize,
    rms: f64,
    psnr_db: f64,
    max_any: f32,
}

fn edge_error(ours: &[f32], reference: &[f32]) -> EdgeError {
    let (mut sum, mut n, mut max_any) = (0.0f64, 0usize, 0.0f32);
    for (a, b) in ours.iter().zip(reference) {
        let e = (a - b).abs();
        max_any = max_any.max(e);
        if *b > 0.0 && *b < 1.0 {
            sum += f64::from(e) * f64::from(e);
            n += 1;
        }
    }
    let rms = (sum / n.max(1) as f64).sqrt();
    EdgeError {
        edge_pixels: n,
        rms,
        psnr_db: -20.0 * rms.log10(),
        max_any,
    }
}

fn star_polygon(w: f64, h: f64, points: u32) -> Vec<(f64, f64)> {
    let outer = w.min(h) / 2.0;
    let inner = outer * 0.46;
    let n = points * 2;
    (0..n)
        .map(|i| {
            let a = -core::f64::consts::FRAC_PI_2
                + f64::from(i) * core::f64::consts::TAU / f64::from(n);
            let r = if i % 2 == 1 { inner } else { outer };
            (a.cos() * r, a.sin() * r)
        })
        .collect()
}

#[test]
fn level2_edges_are_analytic() {
    let mut renderer = Renderer::headless().unwrap();
    let (rotation, scale) = (17.0, 130.0);
    let ellipse = json!({ "shape": "ellipse", "w": 150, "h": 96, "color": "#FFFFFF", "stroke": 0 });
    let star = json!({ "shape": "star", "w": 150, "h": 150, "points": 7, "color": "#FFFFFF", "stroke": 0 });
    let star_poly = star_polygon(150.0, 150.0, 7);
    // A line is a round-capped stroke: a capsule of radius 4.5 around
    // (-80, 0)..(80, 0) in layer units.
    let line = json!({ "shape": "line", "w": 160, "h": 20, "color": "#FFFFFF", "stroke": 9 });
    type Inside = Box<dyn Fn(f64, f64) -> bool>;
    let cases: Vec<(&str, Value, Inside)> = vec![
        (
            "ellipse",
            ellipse,
            Box::new(move |x, y| {
                let (u, v) = to_layer(x, y, rotation, scale);
                (u / 75.0).powi(2) + (v / 48.0).powi(2) <= 1.0
            }),
        ),
        (
            "line",
            line,
            Box::new(move |x, y| {
                let (u, v) = to_layer(x, y, rotation, scale);
                let cu = u.clamp(-80.0, 80.0);
                (u - cu).powi(2) + v.powi(2) <= 4.5 * 4.5
            }),
        ),
        (
            "star",
            star,
            Box::new(move |x, y| {
                let (u, v) = to_layer(x, y, rotation, scale);
                point_in_polygon(&star_poly, u, v)
            }),
        ),
    ];
    for (name, d, inside) in cases {
        let reference = coverage(inside);
        let level2 = render_alpha(
            &mut renderer,
            vec![placed(name, "shape", d.clone(), 2, rotation, scale)],
        );
        let level1 = render_alpha(
            &mut renderer,
            vec![placed(name, "shape", d, 1, rotation, scale)],
        );
        let e2 = edge_error(&level2, &reference);
        let e1 = edge_error(&level1, &reference);
        eprintln!(
            "{name}: level 2 over {} edge pixels: rms {:.5} ({:.1} dB), max {:.4}; level 1 rms {:.5} ({:.1} dB), max {:.4}",
            e2.edge_pixels, e2.rms, e2.psnr_db, e2.max_any, e1.rms, e1.psnr_db, e1.max_any
        );
        assert!(e2.edge_pixels > 300, "{name}: the shape has edges");
        assert!(
            e2.psnr_db >= 40.0,
            "{name}: level 2 edges are analytic ({:.1} dB)",
            e2.psnr_db
        );
        assert!(
            e2.max_any <= 0.03,
            "{name}: level 2 largest error {}",
            e2.max_any
        );
        assert!(
            e2.rms * 2.0 < e1.rms,
            "{name}: level 2 is sharper than the level 1 raster ({:.5} vs {:.5})",
            e2.rms,
            e1.rms
        );
    }
}

#[test]
fn gpu_and_cpu_coverage_agree() {
    let mut renderer = Renderer::headless().unwrap();
    let layers = || {
        vec![
            placed(
                "star",
                "shape",
                json!({ "shape": "star", "w": 180, "h": 180, "points": 9, "color": "#4C8DFF", "stroke": 6, "strokeColor": "#F2C14E" }),
                2,
                -23.0,
                110.0,
            ),
            placed(
                "rect",
                "shape",
                json!({ "shape": "rect", "w": 120, "h": 70, "radius": 18, "color": "#3FCF8E", "stroke": 0 }),
                2,
                41.0,
                90.0,
            ),
        ]
    };
    renderer.set_vector_backend(CoverageBackend::Auto);
    let gpu = render_alpha(&mut renderer, layers());
    assert_eq!(
        renderer.last_vector_backend(),
        Some("vello"),
        "Vello draws on the GPU"
    );
    renderer.set_vector_backend(CoverageBackend::Cpu);
    let cpu = render_alpha(&mut renderer, layers());
    assert_eq!(renderer.last_vector_backend(), Some("vello_cpu"));
    let (mut max, mut sum, mut differing) = (0.0f32, 0.0f64, 0usize);
    for (a, b) in gpu.iter().zip(&cpu) {
        let e = (a - b).abs();
        max = max.max(e);
        sum += f64::from(e);
        differing += usize::from(e > 0.0);
    }
    let mean = sum / gpu.len() as f64;
    eprintln!("Vello GPU vs vello_cpu: {differing} pixels differ, max {max:.4}, mean {mean:.6}");
    assert!(
        max <= 3.0 / 255.0 + 1e-6,
        "coverage agrees within 3/255 (max {max})"
    );
    assert!(mean <= 1e-3, "mean difference {mean}");
}

#[test]
fn solid_radius_is_honored_on_level2_only() {
    let mut renderer = Renderer::headless().unwrap();
    let d = json!({ "color": "#FFFFFF", "w": 120, "h": 80, "radius": 30 });
    let mut l1 = layer("solid", "solid", d.clone(), 60.0, 70.0, 2.0);
    let level1 = render_alpha(&mut renderer, vec![l1.clone()]);
    l1["compat"] = json!(2);
    let level2 = render_alpha(&mut renderer, vec![l1]);
    // The solid's box, as level 1 draws it (today's quad, radius ignored).
    let opaque: Vec<(u32, u32)> = (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .filter(|(x, y)| level1[(y * W + x) as usize] > 0.5)
        .collect();
    let (x0, y0) = (
        opaque.iter().map(|p| p.0).min().unwrap(),
        opaque.iter().map(|p| p.1).min().unwrap(),
    );
    let (x1, y1) = (
        opaque.iter().map(|p| p.0).max().unwrap() + 1,
        opaque.iter().map(|p| p.1).max().unwrap() + 1,
    );
    assert_eq!((x1 - x0, y1 - y0), (120, 80), "level 1 draws the full box");
    let at = |a: &[f32], x: u32, y: u32| a[(y * W + x) as usize];
    assert_eq!(at(&level1, x0, y0), 1.0, "level 1 ignores the radius");
    assert_eq!(at(&level2, x0, y0), 0.0, "level 2 rounds the corner");
    assert_eq!(
        at(&level2, x0 + 60, y0),
        1.0,
        "level 2 keeps the straight edges"
    );
    // Level 2 against the exact rounded rectangle.
    let (fx0, fy0) = (f64::from(x0), f64::from(y0));
    let reference = coverage(|x, y| {
        let (u, v) = (x - fx0, y - fy0);
        if !(0.0..=120.0).contains(&u) || !(0.0..=80.0).contains(&v) {
            return false;
        }
        let cx = u.clamp(30.0, 90.0);
        let cy = v.clamp(30.0, 50.0);
        (u - cx).powi(2) + (v - cy).powi(2) <= 900.0
    });
    let e = edge_error(&level2, &reference);
    eprintln!(
        "solid radius 30: {} edge pixels, rms {:.5} ({:.1} dB), max {:.4}",
        e.edge_pixels, e.rms, e.psnr_db, e.max_any
    );
    assert!(e.psnr_db >= 40.0 && e.max_any <= 0.03);
}

/// The opaque bounds of an alpha frame: `(x0, y0, x1, y1)`.
fn opaque_bounds(a: &[f32]) -> (u32, u32, u32, u32) {
    let opaque: Vec<(u32, u32)> = (0..H)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .filter(|(x, y)| a[(y * W + x) as usize] > 0.5)
        .collect();
    (
        opaque.iter().map(|p| p.0).min().unwrap(),
        opaque.iter().map(|p| p.1).min().unwrap(),
        opaque.iter().map(|p| p.0).max().unwrap() + 1,
        opaque.iter().map(|p| p.1).max().unwrap() + 1,
    )
}

#[test]
fn level2_solids_without_radius_have_analytic_edges() {
    let mut renderer = Renderer::headless().unwrap();
    // Radius 0, a missing radius, and a negative one are plain rectangles.
    for d in [
        json!({ "color": "#FFFFFF", "w": 120, "h": 80, "radius": 0 }),
        json!({ "color": "#FFFFFF", "w": 120, "h": 80 }),
        json!({ "color": "#FFFFFF", "w": 120, "h": 80, "radius": -5 }),
    ] {
        // Where the box sits relative to the layer origin, unrotated.
        let upright = render_alpha(
            &mut renderer,
            vec![placed("solid", "solid", d.clone(), 2, 0.0, 100.0)],
        );
        let (x0, y0, x1, y1) = opaque_bounds(&upright);
        assert_eq!((x1 - x0, y1 - y0), (120, 80), "{d}: the full box");
        let (bx, by) = (f64::from(x0) - 128.0, f64::from(y0) - 128.0);
        let (rotation, scale) = (23.0, 115.0);
        let ours = render_alpha(
            &mut renderer,
            vec![placed("solid", "solid", d.clone(), 2, rotation, scale)],
        );
        let reference = coverage(|x, y| {
            let (u, v) = to_layer(x, y, rotation, scale);
            (bx..=bx + 120.0).contains(&u) && (by..=by + 80.0).contains(&v)
        });
        let e = edge_error(&ours, &reference);
        let partial = ours.iter().filter(|a| **a > 0.02 && **a < 0.98).count();
        eprintln!(
            "level 2 solid {d}: rotated {rotation} at {scale}%: {} edge pixels ({partial} partial), rms {:.5} ({:.1} dB), max {:.4}",
            e.edge_pixels, e.rms, e.psnr_db, e.max_any
        );
        assert!(
            partial * 2 >= e.edge_pixels,
            "{d}: edges are antialiased, not hard ({partial} of {})",
            e.edge_pixels
        );
        assert!(
            e.psnr_db >= 40.0 && e.max_any <= 0.03,
            "{d}: {:.1} dB, max {:.4}",
            e.psnr_db,
            e.max_any
        );
    }
    // For contrast: level 1 keeps today's single-sample quad.
    let level1 = render_alpha(
        &mut renderer,
        vec![placed(
            "solid",
            "solid",
            json!({ "color": "#FFFFFF", "w": 120, "h": 80 }),
            1,
            23.0,
            115.0,
        )],
    );
    assert!(
        level1.iter().all(|a| *a == 0.0 || *a == 1.0),
        "level 1 solids are aliased as today"
    );
}

#[test]
fn vector_cache_stays_bounded_while_content_animates() {
    let mut renderer = Renderer::headless().unwrap();
    let frame = |rotation: f64| {
        vec![placed(
            "s",
            "shape",
            json!({ "shape": "star", "w": 200, "h": 200, "points": 5, "color": "#FFFFFF" }),
            2,
            rotation,
            100.0,
        )]
    };
    // One frame's working set.
    render_alpha(&mut renderer, frame(0.0));
    let one = renderer.vector_cache_bytes();
    assert!(one > 0);
    let budget = one * 3;
    renderer.set_vector_cache_budget(budget, 4096);
    // A rotating level 2 shape misses on every frame. Without trim (one
    // work item, or playback), the cache must stay within its budget plus
    // the frame being drawn.
    let mut peak = 0;
    for i in 1..=40 {
        render_alpha(&mut renderer, frame(f64::from(i) * 3.0));
        peak = peak.max(renderer.vector_cache_bytes());
    }
    eprintln!(
        "rotating level 2 shape: {one} bytes per frame, budget {budget}, peak {peak} after 40 frames, {} entries",
        renderer.vector_cache_len()
    );
    assert!(
        peak <= budget + one,
        "peak {peak} exceeds budget {budget} + one frame {one}"
    );
    assert!(renderer.vector_cache_len() <= 4);
    // The entry bound holds as well.
    renderer.set_vector_cache_budget(u64::MAX, 2);
    assert!(renderer.vector_cache_len() <= 2);
    for i in 0..10 {
        render_alpha(&mut renderer, frame(200.0 + f64::from(i)));
        assert!(renderer.vector_cache_len() <= 2);
    }
    // A frame needing more than the budget keeps its working set and draws
    // every layer; the next frame evicts it.
    renderer.set_vector_cache_budget(1, 4096);
    let many: Vec<Value> = (0..6)
        .map(|i| {
            let mut l = placed(
                &format!("s{i}"),
                "shape",
                json!({ "shape": "ellipse", "w": 30, "h": 30, "color": "#FFFFFF" }),
                2,
                0.0,
                100.0,
            );
            set(&mut l, "position.x", json!(30.0 + 38.0 * f64::from(i)));
            l
        })
        .collect();
    let drawn = render_alpha(&mut renderer, many);
    for i in 0..6u32 {
        let x = 30 + 38 * i;
        assert_eq!(drawn[(128 * W + x) as usize], 1.0, "layer {i} is drawn");
    }
    render_alpha(&mut renderer, frame(300.0));
    assert!(
        renderer.vector_cache_len() <= 1,
        "{} entries",
        renderer.vector_cache_len()
    );
}

#[test]
fn trim_splits_its_budget_with_the_target_pool() {
    let mut renderer = Renderer::headless().unwrap();
    let layers = vec![placed(
        "b",
        "shape",
        json!({ "shape": "polygon", "w": 200, "h": 200, "points": 6, "color": "#FFFFFF" }),
        2,
        10.0,
        100.0,
    )];
    render_alpha(&mut renderer, layers);
    let vector = renderer.vector_cache_bytes();
    let pool = renderer.pooled_bytes();
    assert!(vector > 0 && pool > 0);
    let budget = (vector + pool) / 2;
    renderer.trim(budget);
    assert!(
        renderer.vector_cache_bytes() + renderer.pooled_bytes() <= budget,
        "vector {} + pool {} > {budget}",
        renderer.vector_cache_bytes(),
        renderer.pooled_bytes()
    );
}

#[test]
fn vector_textures_are_accounted_and_trimmed() {
    let mut renderer = Renderer::headless().unwrap();
    assert_eq!(renderer.vector_cache_bytes(), 0);
    let layers = vec![
        placed(
            "a",
            "shape",
            json!({ "shape": "ellipse", "w": 100, "h": 60, "color": "#FFFFFF" }),
            1,
            0.0,
            100.0,
        ),
        placed(
            "b",
            "shape",
            json!({ "shape": "polygon", "w": 100, "h": 100, "points": 6, "color": "#FFFFFF" }),
            2,
            10.0,
            100.0,
        ),
    ];
    let first = render_alpha(&mut renderer, layers.clone());
    let bytes = renderer.vector_cache_bytes();
    eprintln!("vector cache after one frame: {bytes} bytes");
    // A level 1 raster (RGBA8) and a level 2 coverage atlas plus its float
    // paint target hold at least the shapes' pixels.
    assert!(bytes >= 100 * 60 * 4 + 100 * 100 * 4, "{bytes} bytes");
    // The same frame again reuses the cached textures.
    let again = render_alpha(&mut renderer, layers.clone());
    assert_eq!(renderer.vector_cache_bytes(), bytes);
    assert_eq!(first, again);
    // Lease pressure frees them; the next frame rebuilds identical pixels.
    renderer.trim(0);
    assert_eq!(renderer.vector_cache_bytes(), 0);
    let rebuilt = render_alpha(&mut renderer, layers);
    assert_eq!(first, rebuilt);
}

/// An SVG image layer (`d.asset` → `path`), `compat` level `compat`.
fn svg_project(
    svg: &str,
    name: &str,
    w: u32,
    h: u32,
    d: Value,
    compat: u32,
    rotation: f64,
) -> (Value, MapResolver) {
    let dir = std::env::temp_dir().join(format!("pm-render-vector-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    std::fs::write(&path, svg).unwrap();
    let mut doc = project(
        W,
        H,
        2.0,
        no_fill(),
        vec![placed("svg", "image", d, compat, rotation, 100.0)],
    );
    asset(&mut doc, "art", "image", "svg", w, h, 0.0);
    let mut resolver = MapResolver::default();
    resolver.paths.insert("art".into(), path);
    (doc, resolver)
}

const SOLID_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50"><rect x="10" y="10" width="80" height="30" fill="#FFFFFF"/><circle cx="50" cy="25" r="8" fill="#000000"/></svg>"##;

#[test]
fn level2_svg_images_are_vectors_at_output_resolution() {
    let mut renderer = Renderer::headless().unwrap();
    let d = json!({ "asset": "art", "fit": "stretch", "w": 200, "h": 100 });
    let (doc, resolver) = svg_project(SOLID_SVG, "solid.svg", 100, 50, d.clone(), 2, 23.0);
    assert_eq!(
        unsupported_with_media(&load(&doc), "Ptest-root", &resolver),
        vec![]
    );
    let rgba = render_rgba(&mut renderer, doc, &resolver);
    let alpha: Vec<f32> = rgba.chunks(4).map(|p| p[3]).collect();
    // The 200 × 100 box centered at (128, 128) rotated 23°: user units ×2.
    let reference = coverage(|x, y| {
        let (u, v) = to_layer(x, y, 23.0, 100.0);
        let (sx, sy) = ((u + 100.0) / 2.0, (v + 50.0) / 2.0);
        (10.0..=90.0).contains(&sx) && (10.0..=40.0).contains(&sy)
    });
    let e = edge_error(&alpha, &reference);
    eprintln!(
        "SVG rect: {} edge pixels, rms {:.5} ({:.1} dB), max {:.4}",
        e.edge_pixels, e.rms, e.psnr_db, e.max_any
    );
    assert!(e.psnr_db >= 40.0 && e.max_any <= 0.03);
    // The circle is black and opaque over the white rect.
    let center = (128 * W + 128) as usize * 4;
    assert!(
        rgba[center] < 1e-3 && (rgba[center + 3] - 1.0).abs() < 1e-3,
        "{:?}",
        &rgba[center..center + 4]
    );
    // Level 1 SVG images are drawn (Chromium's rasters; tests/svg_level1.rs).
    let (doc1, resolver1) = svg_project(SOLID_SVG, "solid1.svg", 100, 50, d, 1, 23.0);
    assert_eq!(
        unsupported_with_media(&load(&doc1), "Ptest-root", &resolver1),
        vec![]
    );
}

#[test]
fn level2_svg_fallbacks_and_text() {
    let mut renderer = Renderer::headless().unwrap();
    // A gradient is drawn by resvg; `cover` crops the 2:1 SVG to the
    // square box.
    let gradient = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50"><defs><linearGradient id="g"><stop offset="0" stop-color="#FFFFFF"/><stop offset="1" stop-color="#FFFFFF" stop-opacity="0.5"/></linearGradient></defs><rect width="100" height="50" fill="url(#g)"/></svg>"##;
    let d = json!({ "asset": "art", "fit": "cover", "w": 100, "h": 100 });
    let (doc, resolver) = svg_project(gradient, "gradient.svg", 100, 50, d, 2, 0.0);
    assert_eq!(
        unsupported_with_media(&load(&doc), "Ptest-root", &resolver),
        vec![]
    );
    let rgba = render_rgba(&mut renderer, doc, &resolver);
    let a = |x: u32, y: u32| rgba[((y * W + x) * 4 + 3) as usize];
    // Covered exactly over the 100 × 100 box at (78..178, 78..178).
    assert_eq!(a(77, 128), 0.0);
    assert_eq!(a(178, 128), 0.0);
    assert!(a(78, 128) > 0.5 && a(177, 128) > 0.5);
    assert_eq!(a(128, 77), 0.0);
    assert!(a(128, 78) > 0.5);
    // The gradient's middle half shows: alpha 0.875 at the left edge,
    // 0.625 at the right.
    assert!((a(79, 128) - 0.87).abs() < 0.02, "{}", a(79, 128));
    assert!((a(176, 128) - 0.63).abs() < 0.02, "{}", a(176, 128));
    // SVG text is routed.
    let text = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50"><text x="5" y="30">Powermove</text></svg>"##;
    let (doc, resolver) = svg_project(
        text,
        "text.svg",
        100,
        50,
        json!({ "asset": "art", "w": 100, "h": 50 }),
        2,
        0.0,
    );
    let u = unsupported_with_media(&load(&doc), "Ptest-root", &resolver);
    assert_eq!(
        u.iter().map(|u| u.feature.clone()).collect::<Vec<_>>(),
        vec![Feature::SvgImage]
    );
}

/// SVG bytes in another encoding Chromium honors (an XML declaration's
/// ISO-8859-1, UTF-16 with a byte order mark) are valid SVG images: level
/// 1 asks the renderer for Chromium's raster of the bytes as they are (no
/// layer failure), level 2 decodes them as Chromium does and draws the
/// same pixels as the UTF-8 file.
#[test]
fn svg_images_in_declared_encodings_are_not_failures() {
    let mut renderer = Renderer::headless().unwrap();
    let utf8 = r##"<?xml version="1.0" encoding="UTF-8"?>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 50"><title>Café</title><rect x="10" y="10" width="80" height="30" fill="#FFFFFF"/></svg>"##;
    let latin1: Vec<u8> = utf8
        .replace("UTF-8", "ISO-8859-1")
        .chars()
        .map(|c| c as u32 as u8)
        .collect();
    assert!(std::str::from_utf8(&latin1).is_err());
    let utf16 = {
        let text = utf8.replace("UTF-8", "UTF-16");
        let mut b = vec![0xFF, 0xFE];
        b.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        b
    };
    let d = json!({ "asset": "art", "fit": "stretch", "w": 200, "h": 100 });
    let (doc, resolver) = svg_project(utf8, "declared-utf8.svg", 100, 50, d.clone(), 2, 0.0);
    let reference = render_rgba(&mut renderer, doc, &resolver);
    for (name, bytes) in [("latin1.svg", &latin1), ("utf16.svg", &utf16)] {
        let (doc1, resolver1) = svg_project(utf8, name, 100, 50, d.clone(), 1, 0.0);
        std::fs::write(resolver1.paths["art"].clone(), bytes).unwrap();
        assert_eq!(
            unsupported_with_media(&load(&doc1), "Ptest-root", &resolver1),
            vec![],
            "{name}"
        );
        // Rendering asks for the raster, never reports a corrupt file.
        let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
        let frame = renderer
            .render(&load(&doc1), "Ptest-root", ticks(0.0), &request, &resolver1)
            .unwrap();
        assert!(frame.failures.is_empty(), "{name}: {:?}", frame.failures);
        assert_eq!(frame.svg_misses.len(), 1, "{name}");

        let (doc2, resolver2) = svg_project(utf8, name, 100, 50, d.clone(), 2, 0.0);
        std::fs::write(resolver2.paths["art"].clone(), bytes).unwrap();
        assert_eq!(
            unsupported_with_media(&load(&doc2), "Ptest-root", &resolver2),
            vec![],
            "{name}"
        );
        assert!(
            render_rgba(&mut renderer, doc2, &resolver2) == reference,
            "{name}"
        );
    }
}

/// Level 1 SVGs that roxmltree rejects but Chromium's libxml2 paints (an
/// external DTD subset with undeclared entities, a declared external
/// entity, a parameter entity, nesting at Blink's 5,000-element limit) are
/// Chromium's rasters like any other: the frame asks the renderer for
/// them, never paints them empty. Documents Chromium shows as a broken
/// `<img>` (an undeclared entity without a DTD, nesting past the limit)
/// draw nothing and ask for nothing, as today.
#[test]
fn level1_svgs_libxml2_accepts_are_asked_for_not_blank() {
    let mut renderer = Renderer::headless().unwrap();
    let r = r#"<rect width="8" height="8" fill="red"/>"#;
    let ns = r#"xmlns="http://www.w3.org/2000/svg" width="8" height="8""#;
    let nest = |levels: usize| {
        format!(
            "<svg {ns}>{}{r}{}</svg>",
            "<g>".repeat(levels - 1),
            "</g>".repeat(levels - 1)
        )
    };
    let painted = [
        format!(
            r#"<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd"><svg {ns}>{r}<text>&nbsp;&copy;</text></svg>"#
        ),
        format!(r#"<!DOCTYPE svg SYSTEM "svg.dtd"><svg {ns}>{r}<text>&foo;</text></svg>"#),
        format!(r#"<!DOCTYPE svg [<!ENTITY e SYSTEM "e.xml">]><svg {ns}>{r}&e;</svg>"#),
        format!(
            r#"<!DOCTYPE svg [<!ENTITY % a "<!ENTITY b 'x'>"> %a;]><svg {ns}>{r}<text>&b;</text></svg>"#
        ),
        nest(4999),
    ];
    let broken = [
        format!("<svg {ns}>{r}<text>&nbsp;</text></svg>"),
        format!(
            r#"<?xml version="1.0" standalone="yes"?><!DOCTYPE svg SYSTEM "svg.dtd"><svg {ns}>{r}<text>&foo;</text></svg>"#
        ),
        nest(5000),
    ];
    let d = json!({ "asset": "art", "fit": "stretch", "w": 64, "h": 64 });
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float32);
    for (i, svg) in painted.iter().chain(&broken).enumerate() {
        let name = format!("libxml2-{i}.svg");
        let (doc, resolver) = svg_project(svg, &name, 8, 8, d.clone(), 1, 0.0);
        let doc = load(&doc);
        let u = unsupported_with_media(&doc, "Ptest-root", &resolver);
        let result = renderer.render(&doc, "Ptest-root", ticks(0.0), &request, &resolver);
        assert_eq!(u, vec![], "{name}");
        let frame = result.unwrap();
        assert!(frame.failures.is_empty(), "{name}");
        if i < painted.len() {
            assert_eq!(frame.svg_misses.len(), 1, "{name}");
        } else {
            assert!(frame.svg_misses.is_empty(), "{name}");
            assert!(
                renderer
                    .read_rgba(&frame)
                    .unwrap()
                    .iter()
                    .all(|&v| v == 0.0),
                "{name}"
            );
        }
    }
}

/// A level 1 path layer (an editable triangle) at `(x, y)`, its first
/// vertex moving with time (an animated path misses the cache every frame).
fn path_layer(id: &str, x: f64, y: f64, spin: f64) -> Value {
    let ch = |v: Value| json!({ "v": v, "kf": [], "expr": null });
    let moving = json!({ "v": -40, "expr": null, "kf": [
        { "t": 0, "v": -40, "ease": "linear" }, { "t": 2, "v": -40.0 + spin, "ease": "linear" }
    ] });
    let mut l = layer(
        id,
        "shape",
        json!({ "shape": "rect", "w": 120.0, "h": 120.0, "color": "#FFFFFF", "paths": [{
            "id": "path1", "name": "Tri", "parent": null,
            "p": {
                "x": ch(json!(0)), "y": ch(json!(0)), "rotation": ch(json!(spin)),
                "scaleX": ch(json!(100)), "scaleY": ch(json!(100)), "closed": ch(json!(true)),
                "fill": ch(json!("#3A86FF")), "fillEnabled": ch(json!(true)), "fillOpacity": ch(json!(100)),
                "stroke": ch(json!("#FF006E")), "strokeWidth": ch(json!(3)), "strokeOpacity": ch(json!(80)),
                "trimStart": ch(json!(0)), "trimEnd": ch(json!(100)), "trimOffset": ch(json!(0)),
                "copies": ch(json!(1)), "repeatX": ch(json!(30)), "repeatY": ch(json!(0)), "repeatRotation": ch(json!(0))
            },
            "vertices": [
                { "id": "v1", "p": { "x": moving, "y": ch(json!(-30)), "inX": ch(json!(0)), "inY": ch(json!(0)), "outX": ch(json!(0)), "outY": ch(json!(0)) } },
                { "id": "v2", "p": { "x": ch(json!(40)), "y": ch(json!(-30)), "inX": ch(json!(0)), "inY": ch(json!(0)), "outX": ch(json!(0)), "outY": ch(json!(0)) } },
                { "id": "v3", "p": { "x": ch(json!(0)), "y": ch(json!(40)), "inX": ch(json!(0)), "inY": ch(json!(0)), "outX": ch(json!(0)), "outY": ch(json!(0)) } }
            ]
        }] }),
        x,
        y,
        2.0,
    );
    set(&mut l, "rotation", json!(spin / 3.0));
    l
}

#[test]
fn level1_rasters_drawn_ahead_on_workers_are_the_rasters_drawn_in_order() {
    // Many animated level 1 shapes and paths, top level, in a nested
    // composition, and under motion blur: every frame misses the cache for
    // all of them. Drawn ahead on worker threads, the frame is byte for
    // byte the frame drawn in order.
    let mut layers: Vec<Value> = Vec::new();
    for i in 0..10 {
        let shape = ["rect", "ellipse", "star", "polygon"][i % 4];
        let color = ["#FFFFFF", "#80FFDB", "#F72585", "#4CC9F0"][i % 4];
        let mut l = layer(
            &format!("s{i}"),
            "shape",
            json!({ "shape": shape, "w": 40 + 7 * i, "h": 30 + 5 * i, "points": 5 + i % 3,
                    "radius": 6, "stroke": 2, "strokeColor": "#FFD60A",
                    "color": color }),
            20.0 + 22.0 * i as f64,
            40.0 + 15.0 * (i % 5) as f64,
            2.0,
        );
        set(&mut l, "scale.x", json!({ "v": 100, "expr": null, "kf": [
            { "t": 0, "v": 60, "ease": "linear" }, { "t": 2, "v": 180, "ease": "linear" }
        ] }));
        if i % 3 == 0 {
            l["mblur"] = json!(true);
            set(&mut l, "position.x", json!({ "v": 0, "expr": null, "kf": [
                { "t": 0, "v": 0, "ease": "linear" }, { "t": 2, "v": 256, "ease": "linear" }
            ] }));
        }
        layers.push(l);
    }
    for i in 0..4 {
        layers.push(path_layer(&format!("p{i}"), 60.0 + 45.0 * f64::from(i), 190.0, 25.0 + 20.0 * f64::from(i)));
    }
    layers.push(layer("pre", "precomp", json!({ "comp": "inner", "w": W, "h": H }), 128.0, 128.0, 2.0));
    let mut doc = project(W, H, 2.0, no_fill(), layers);
    composition(
        &mut doc,
        "inner",
        W,
        H,
        2.0,
        (0..3).map(|i| path_layer(&format!("n{i}"), 70.0 + 50.0 * f64::from(i), 70.0, 40.0)).collect(),
    );
    let doc = load(&doc);
    let mut request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    request.motion_blur = pm_render::motion::MotionBlur::Samples(6);
    let draw = |renderer: &mut Renderer, t: f64| {
        let frame = renderer
            .render(&doc, "Ptest-root", ticks(t), &request, &MapResolver::default())
            .expect("render");
        assert!(frame.failures.is_empty(), "{:?}", frame.failures);
        (renderer.read_rgba(&frame).unwrap(), renderer.last_stages())
    };
    let mut ahead = Renderer::headless().unwrap();
    let mut in_order = Renderer::headless().unwrap();
    in_order.set_vector_prefetch(false);
    let mut drawn_ahead = 0;
    for t in [0.0, 0.37, 0.91, 1.5] {
        let (a, stages) = draw(&mut ahead, t);
        let (b, plain) = draw(&mut in_order, t);
        assert_eq!(plain.prefetched, 0);
        drawn_ahead += stages.prefetched;
        assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "frame at {t} differs");
        assert!(a.chunks(4).any(|p| p[3] > 0.0), "the frame draws something at {t}");
        assert_eq!(ahead.vector_cache_len(), in_order.vector_cache_len(), "same cache entries at {t}");
    }
    assert!(drawn_ahead >= 40, "rasters drawn ahead: {drawn_ahead}");
}

/// Bigrender round 3: a playing view plans the frames it will draw next
/// and their level 1 rasters are drawn ahead on threads of their own
/// (`Renderer::plan_ahead`); the frames take them and are the same pixels
/// as frames that drew their own.
#[test]
fn rasters_drawn_ahead_of_a_frame_are_taken_by_it_and_draw_the_same_pixels() {
    let mut layers: Vec<Value> = Vec::new();
    for i in 0..8 {
        let shape = ["rect", "ellipse", "star", "polygon"][i % 4];
        let mut l = layer(
            &format!("s{i}"),
            "shape",
            json!({ "shape": shape, "w": 40 + 7 * i, "h": 30 + 5 * i, "points": 5 + i % 3,
                    "radius": 6, "stroke": 2, "strokeColor": "#FFD60A", "color": "#F72585" }),
            20.0 + 25.0 * i as f64,
            60.0 + 15.0 * (i % 5) as f64,
            2.0,
        );
        set(&mut l, "scale.x", json!({ "v": 100, "expr": null, "kf": [
            { "t": 0, "v": 60, "ease": "linear" }, { "t": 2, "v": 180, "ease": "linear" }
        ] }));
        layers.push(l);
    }
    layers.push(path_layer("p0", 80.0, 190.0, 30.0));
    let doc = load(&project(W, H, 2.0, no_fill(), layers));
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    let draw = |renderer: &mut Renderer, t: f64| {
        let frame = renderer
            .render(&doc, "Ptest-root", ticks(t), &request, &MapResolver::default())
            .expect("render");
        renderer.read_rgba(&frame).unwrap()
    };
    let mut ahead = Renderer::headless().unwrap();
    let mut own = Renderer::headless().unwrap();
    let times = [0.0, 0.25, 0.5, 0.75, 1.0];
    // The first frame, then each frame plans the one after it.
    for (i, t) in times.iter().enumerate() {
        if let Some(next) = times.get(i + 1) {
            let needs = ahead.plan_ahead(&doc, "Ptest-root", ticks(*next), &request, &MapResolver::default(), true);
            assert!(needs.is_empty());
        }
        // Let the threads finish what they can (a frame waits for the rest).
        std::thread::sleep(std::time::Duration::from_millis(20));
        let a = draw(&mut ahead, *t);
        let b = draw(&mut own, *t);
        assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "frame at {t} differs");
        assert_eq!(ahead.vector_cache_len(), own.vector_cache_len(), "same cache entries at {t}");
    }
    let stats = ahead.vector_ahead_stats();
    // Four frames planned ahead, five animated rasters each.
    assert!(stats.drawn >= 16, "{stats:?}");
    assert_eq!(stats.taken, stats.drawn, "the frames took everything drawn ahead: {stats:?}");
    // A seek: what was drawn for frames that will not come is dropped.
    ahead.plan_ahead(&doc, "Ptest-root", ticks(1.7), &request, &MapResolver::default(), true);
    std::thread::sleep(std::time::Duration::from_millis(50));
    ahead.forget_ahead();
    assert_eq!(ahead.vector_ahead_stats().bytes, 0);
}

/// A raster override that has not answered a frame planned ahead: its
/// layers are planned as pass-throughs (their rasters drawn ahead) and named
/// as needs; once the producer passes them through, the frame takes them.
#[test]
fn rasters_are_drawn_ahead_for_layers_a_producer_has_not_answered_yet() {
    use pm_render::compat::{CompatAnswer, CompatRasters, CompatSource};
    struct Producer(std::sync::atomic::AtomicBool);
    impl CompatRasters for Producer {
        fn producing(&self, _: CompatSource) -> bool {
            true
        }
        fn answer(&self, _: &str, _: &str, _: f64, _: f64, _: CompatSource, _: Option<pm_protocol::common::PixelRect>) -> CompatAnswer {
            if self.0.load(std::sync::atomic::Ordering::Relaxed) { CompatAnswer::Native } else { CompatAnswer::Missing }
        }
    }
    let mut layers: Vec<Value> = Vec::new();
    for i in 0..6 {
        let mut l = layer(
            &format!("s{i}"),
            "shape",
            json!({ "shape": "star", "w": 40 + 7 * i, "h": 40, "points": 5, "radius": 6,
                    "stroke": 2, "strokeColor": "#FFD60A", "color": "#F72585" }),
            30.0 + 30.0 * i as f64,
            80.0,
            2.0,
        );
        set(&mut l, "scale.x", json!({ "v": 100, "expr": null, "kf": [
            { "t": 0, "v": 60, "ease": "linear" }, { "t": 2, "v": 180, "ease": "linear" }
        ] }));
        layers.push(l);
    }
    let doc = load(&project(W, H, 2.0, no_fill(), layers));
    let request = RenderRequest::at_scale(Size::new(W, H), 1.0, Precision::Float16);
    let producer = std::sync::Arc::new(Producer(std::sync::atomic::AtomicBool::new(false)));
    let mut ahead = Renderer::headless().unwrap();
    ahead.set_compat_rasters(Some(producer.clone()));
    let needs = ahead.plan_ahead(&doc, "Ptest-root", ticks(0.5), &request, &MapResolver::default(), true);
    assert!(!needs.is_empty(), "the unanswered layers are named");
    std::thread::sleep(std::time::Duration::from_millis(50));
    assert!(ahead.vector_ahead_stats().drawn >= 4, "{:?}", ahead.vector_ahead_stats());
    // The producer passes them through: the frame takes what was drawn.
    producer.0.store(true, std::sync::atomic::Ordering::Relaxed);
    let frame = ahead.render(&doc, "Ptest-root", ticks(0.5), &request, &MapResolver::default()).expect("render");
    let a = ahead.read_rgba(&frame).unwrap();
    let mut own = Renderer::headless().unwrap();
    let frame = own.render(&doc, "Ptest-root", ticks(0.5), &request, &MapResolver::default()).expect("render");
    let b = own.read_rgba(&frame).unwrap();
    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()), "same pixels");
    let stats = ahead.vector_ahead_stats();
    assert_eq!(stats.taken, stats.drawn, "{stats:?}");
}

/// Bigrender round 2: a large paths layer's own raster is drawn only over
/// the window a frame samples (today draws the whole bitmap; the window's
/// pixels are the bitmap's there), in viewer frames and exports alike: the
/// frame is the same pixels as with the whole bitmap.
#[test]
fn a_large_paths_layer_draws_only_the_window_a_frame_samples() {
    let mut big = path_layer("big", 128.0, 128.0, 25.0);
    // Scale the paths up so the bitmap is far larger than the frame.
    set(&mut big, "scale.x", json!(1400));
    set(&mut big, "scale.y", json!(1100));
    let doc = load(&project(256, 256, 2.0, no_fill(), vec![big]));
    for (export, region) in [
        (true, None),
        (false, Some(pm_protocol::common::PixelRect { x: 0, y: 0, width: 256, height: 256 })),
        (false, Some(pm_protocol::common::PixelRect { x: 100, y: 40, width: 96, height: 128 })),
    ] {
        let mut request = RenderRequest::at_scale(Size::new(256, 256), 1.0, Precision::Float32);
        request.export = export;
        request.region = region;
        let draw = |r: &mut Renderer| {
            let frame = r
                .render(&doc, "Ptest-root", ticks(0.7), &request, &MapResolver::default())
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            (r.read_rgba(&frame).unwrap(), r.memory().vector_cache)
        };
        let mut clipped = Renderer::headless().unwrap();
        let (a, clipped_bytes) = draw(&mut clipped);
        let mut whole = Renderer::headless().unwrap();
        whole.set_shape_clipping(false);
        let (b, whole_bytes) = draw(&mut whole);
        let max = a.iter().zip(&b).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
        assert_eq!(max, 0.0, "export {export} {region:?}");
        assert!(a.iter().any(|v| *v > 0.0));
        assert!(clipped_bytes * 2 < whole_bytes, "{clipped_bytes} vs {whole_bytes}");
    }
}
