//! SVG-heavy per-frame render cost (headless): 20 SVG image layers
//! (gradients, clip paths, dashes, group opacity, masks, plain icons) over a
//! 1920 × 1080 frame, animated (a) by position and rotation, (b) by scale.
//! Prints cold first frame, mean and p95 of 60 frames per scenario.

mod support;

use pm_protocol::color::Precision;
use pm_protocol::common::Size;
use pm_render::{MapResolver, RenderRequest, Renderer};
use serde_json::{Value, json};
use support::doc::*;
use support::ticks;

const ART: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180" viewBox="0 0 320 180">
  <defs>
    <linearGradient id="a" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#20304F"/><stop offset="0.5" stop-color="#5A93F2"/><stop offset="1" stop-color="#F7D774"/></linearGradient>
    <radialGradient id="b" cx="0.5" cy="0.5" r="0.5"><stop offset="0" stop-color="#FFF2C8"/><stop offset="1" stop-color="#F06A22" stop-opacity="0"/></radialGradient>
    <clipPath id="c"><rect x="6" y="6" width="308" height="168" rx="20"/></clipPath>
  </defs>
  <g clip-path="url(#c)">
    <rect width="320" height="180" fill="url(#a)"/>
    <circle cx="220" cy="64" r="50" fill="url(#b)"/>
    <path d="M0 140 C 50 100, 120 170, 180 120 S 280 90, 320 130 L 320 180 L 0 180 Z" fill="#101218" opacity="0.8"/>
    <g transform="translate(80 70) rotate(-10)"><rect x="-34" y="-20" width="68" height="40" rx="7" fill="none" stroke="#ECE6D2" stroke-width="5" stroke-dasharray="11 5"/></g>
  </g>
</svg>"##;

const ICON: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 96 96">
  <circle cx="48" cy="48" r="44" fill="#2D6CDF"/>
  <path d="M28 50 L42 64 L70 34" fill="none" stroke="#FFFFFF" stroke-width="9" stroke-linecap="round" stroke-linejoin="round"/>
  <rect x="10" y="80" width="76" height="8" rx="4" fill="#0E2A5C"/>
  <polygon points="48,6 54,18 42,18" fill="#FFC23D"/>
</svg>"##;

const MASKED: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200">
  <defs>
    <radialGradient id="g" cx="100" cy="100" r="90" gradientUnits="userSpaceOnUse"><stop offset="0" stop-color="#7CF3C1"/><stop offset="1" stop-color="#14605A"/></radialGradient>
    <mask id="m"><rect width="200" height="200" fill="#000"/><circle cx="100" cy="100" r="80" fill="#fff"/><circle cx="100" cy="100" r="40" fill="#555"/></mask>
  </defs>
  <g opacity="0.9" mask="url(#m)"><rect width="200" height="200" fill="url(#g)"/><path d="M0 0 L200 200 M200 0 L0 200" stroke="#0B1F1D" stroke-width="12"/></g>
</svg>"##;

fn scenario(animate_scale: bool) -> (pm_model::Document, MapResolver, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("pm-render-svg-perf-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut resolver = MapResolver::default();
    let files = [("art", ART, 320, 180), ("icon", ICON, 96, 96), ("masked", MASKED, 200, 200)];
    let mut layers = Vec::new();
    for i in 0..20 {
        let (name, _, w, h) = files[if i < 8 { 0 } else if i < 16 { 1 } else { 2 }];
        let (x, y) = (120.0 + (i % 5) as f64 * 400.0, 140.0 + (i / 5) as f64 * 260.0);
        let mut l = layer(
            &format!("l{i}"),
            "image",
            json!({ "asset": name, "fit": "contain", "w": w as f64 * 0.9, "h": h as f64 * 0.9 }),
            x,
            y,
            2.0,
        );
        if std::env::var_os("PMDBG_STATIC").is_some() {
            set(&mut l, "scale.x", json!(110.0 + i as f64 * 3.0));
            set(&mut l, "scale.y", json!(110.0 + i as f64 * 3.0));
            set(&mut l, "rotation", json!(10.0 + i as f64));
        } else if animate_scale {
            let s0 = 60.0 + (i % 4) as f64 * 10.0;
            set(&mut l, "scale.x", keys(json!(s0), &[(0.0, json!(s0)), (2.0, json!(s0 * 2.5))]));
            set(&mut l, "scale.y", keys(json!(s0), &[(0.0, json!(s0)), (2.0, json!(s0 * 2.5))]));
        } else {
            set(&mut l, "scale.x", json!(110.0 + i as f64 * 3.0));
            set(&mut l, "scale.y", json!(110.0 + i as f64 * 3.0));
            set(&mut l, "position.x", keys(json!(x), &[(0.0, json!(x)), (2.0, json!(x + 173.3))]));
            set(&mut l, "rotation", keys(json!(0), &[(0.0, json!(0.0)), (2.0, json!(37.0 + i as f64))]));
        }
        layers.push(l);
    }
    let mut doc: Value = project(1920, 1080, 2.0, solid_fill("#202020"), layers);
    for (name, svg, w, h) in files {
        let p = dir.join(format!("{name}.svg"));
        std::fs::write(&p, svg).unwrap();
        asset(&mut doc, name, "image", "svg", w, h, 0.0);
        resolver.paths.insert(name.into(), p);
    }
    (load(&doc), resolver, dir)
}

#[test]
fn svg_heavy_frames() {
    for animate_scale in [false, true] {
        let (doc, resolver, dir) = scenario(animate_scale);
        let mut renderer = Renderer::headless().unwrap();
        
        let request = RenderRequest::at_scale(Size::new(1920, 1080), 1.0, Precision::Float16);
        let mut times = Vec::new();
        let frames: usize = std::env::var("PMDBG_FRAMES").ok().and_then(|v| v.parse().ok()).unwrap_or(61);
        for f in 0..frames {
            let t0 = std::time::Instant::now();
            let frame = renderer
                .render(&doc, "Ptest-root", ticks((f % 61) as f64 / 30.0), &request, &resolver)
                .unwrap();
            assert!(frame.failures.is_empty(), "{:?}", frame.failures);
            let cpu = t0.elapsed().as_secs_f64() * 1e3;
            renderer.wait_idle();
            if std::env::var_os("PMDBG_SPLIT").is_some() && f > 0 {
                eprintln!("split cpu {cpu:.2} total {:.2}", t0.elapsed().as_secs_f64() * 1e3);
            }
            times.push(t0.elapsed().as_secs_f64() * 1e3);
        }
        let cold = times.remove(0);
        let mean = times.iter().sum::<f64>() / times.len() as f64;
        let mut sorted = times.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p95 = sorted[(sorted.len() * 95) / 100];
        let max = sorted[sorted.len() - 1];
        println!(
            "SVG perf {}: cold {cold:.2} ms, mean {mean:.2} ms, p95 {p95:.2} ms, max {max:.2} ms (60 frames)",
            if animate_scale { "animated scale" } else { "animated position+rotation" }
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
