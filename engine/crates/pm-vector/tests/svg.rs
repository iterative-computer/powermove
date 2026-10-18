//! SVG bucket rasters against Chromium's `<img>` rasters
//! (docs/engine/vector.md, "SVG images").
//!
//! The references are `rasterSvgAsset`'s canvases (`drawImage(img, 0, 0,
//! width, height)`) captured by `tools/canvas-ref` (see `tests/canvas2d.rs`
//! for where the capture lives; the test passes with a note without one).
//! `SvgDocument::raster_compat` replays Blink's SVG paint record through the
//! Graphite device emulation. The cases inside the 2D class (at most 1 code
//! value on 99.9% of pixels, at most 4 anywhere) must stay inside it; the
//! remaining cases pin their measured residual so a change is seen (they
//! are blockers listed in the docs, not accepted differences). Text cases
//! need a pm-text resolver and are measured by `examples/svg_compat_compare`.

mod canvas_cases;

use pm_vector::render::svg::SvgDocument;

/// Cases outside the class: (case id, largest difference, pixels off by
/// more than 1), measured on `canvas-20261016-075306-cbd1dcfd`.
const RESIDUAL: &[(&str, u8, usize)] = &[
    // Analytic draws sharing a multisampled pass with a tessellated draw:
    // isolated pixels where Chromium's pixel takes one mesh triangle.
    ("svg-solid-333x150", 44, 4),
    ("svg-stroke-64x64", 15, 30),
    ("svg-stroke-333x150", 19, 135),
    // Gaussian blur: within 2 code values, too many pixels at 2.
    ("svg-blur-64x64", 2, 14),
    ("svg-blur-200x200", 2, 3804),
    ("svg-blur-333x150", 2, 5008),
];

#[test]
fn bucket_rasters_against_chromium() {
    let Some(dir) = canvas_cases::capture_dir() else {
        eprintln!("note: no canvas-* capture (run tools/canvas-ref); SVG rasters not measured");
        return;
    };
    let mut failures = Vec::new();
    let mut compared = 0;
    let mut in_class = 0;
    for c in canvas_cases::cases() {
        if c["kind"] != "svg" {
            continue;
        }
        let id = c["id"].as_str().unwrap();
        let doc = SvgDocument::parse(c["svg"].as_str().unwrap().as_bytes()).unwrap();
        if doc.has_text() {
            eprintln!("{id:28} has text (measured by svg_compat_compare with pm-text)");
            continue;
        }
        let (rw, rh, reference) = canvas_cases::read_png(&dir.join(format!("{id}.png")));
        let ours = doc.raster_compat(rw, rh).unwrap_or_else(|e| panic!("{id}: {e}"));
        let (mut max, mut over1) = (0u8, 0usize);
        for (a, b) in ours.rgba.chunks(4).zip(reference.chunks(4)) {
            let e = a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap();
            max = max.max(e);
            over1 += usize::from(e > 1);
        }
        compared += 1;
        let pixels = (rw * rh) as usize;
        let class = max <= 4 && over1 * 1000 <= pixels;
        eprintln!("{id:28} max {max} over1 {over1}/{pixels} {}", if class { "2D class" } else { "outside" });
        match RESIDUAL.iter().find(|r| r.0 == id) {
            None if class => in_class += 1,
            None => failures.push(format!("{id}: left the 2D class (max {max}, {over1} pixels over 1)")),
            Some(&(_, m, n)) if class => failures.push(format!("{id}: now in the 2D class; drop its residual pin ({m}, {n})")),
            Some(&(_, m, n)) if max <= m && over1 <= n => {}
            Some(&(_, m, n)) => failures.push(format!("{id}: max {max} (pinned {m}), {over1} over 1 (pinned {n})")),
        }
    }
    assert_eq!(compared, 24, "the capture covers the SVG cases without text");
    assert_eq!(in_class, 24 - RESIDUAL.len());
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
