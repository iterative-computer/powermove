//! SVG bucket rasters against Chromium's `<img>` rasters
//! (docs/engine/vector.md, "SVG images").
//!
//! The references are `rasterSvgAsset`'s canvases (`drawImage(img, 0, 0,
//! width, height)`) captured by `tools/canvas-ref` (see `tests/canvas2d.rs`
//! for where the capture lives; the test passes with a note without one).
//! resvg is not Chromium: edges differ where Skia multisamples (one sample is
//! 64 code values) and blurs differ by a few code values. The measured
//! residuals are pinned per SVG so a change is seen; they are outside the 2D
//! parity class, which is why level 1 SVG images stay routed.

mod canvas_cases;

use pm_vector::render::svg::SvgDocument;

/// (SVG, largest share of pixels off by more than 1 code value, largest
/// mean difference), measured on `canvas-20261016-075306-cbd1dcfd` over
/// 64 × 64, 200 × 200, and 333 × 150 buckets.
const MEASURED: &[(&str, f64, f64)] = &[
    ("solid", 0.0404, 0.82),
    ("stroke", 0.130, 2.87),
    ("opacity", 0.0274, 0.45),
    ("gradient", 0.0545, 0.72),
    ("clip", 0.0343, 0.43),
    ("evenodd", 0.0480, 1.27),
    ("blur", 0.186, 0.83),
    ("transform", 0.0504, 1.12),
];

#[test]
fn bucket_rasters_against_chromium() {
    let Some(dir) = canvas_cases::capture_dir() else {
        eprintln!("note: no canvas-* capture (run tools/canvas-ref); SVG rasters not measured");
        return;
    };
    let mut failures = Vec::new();
    let mut compared = 0;
    for c in canvas_cases::cases() {
        if c["kind"] != "svg" {
            continue;
        }
        let id = c["id"].as_str().unwrap();
        let name = id.trim_start_matches("svg-").rsplit_once('-').unwrap().0;
        let doc = SvgDocument::parse(c["svg"].as_str().unwrap().as_bytes()).unwrap();
        if doc.has_text() {
            eprintln!("{id:28} has text (not drawn: SVG text goes through pm-text)");
            continue;
        }
        let (rw, rh, reference) = canvas_cases::read_png(&dir.join(format!("{id}.png")));
        let ours = doc.raster(rw, rh);
        let (mut max, mut over1, mut sum) = (0i32, 0usize, 0f64);
        for (a, b) in ours.rgba.chunks(4).zip(reference.chunks(4)) {
            let e = a.iter().zip(b).map(|(x, y)| (i32::from(*x) - i32::from(*y)).abs()).max().unwrap();
            max = max.max(e);
            over1 += usize::from(e > 1);
            sum += f64::from(e);
        }
        let n = f64::from(rw * rh);
        let (share, mean) = (over1 as f64 / n, sum / n);
        compared += 1;
        eprintln!("{id:28} >1 {:6.2}% mean {mean:.3} max {max}", 100.0 * share);
        match MEASURED.iter().find(|m| m.0 == name) {
            Some(&(_, s, m)) if share <= s + 1e-4 && mean <= m + 1e-3 => {}
            Some(&(_, s, m)) => failures.push(format!("{id}: >1 {share:.4} (pinned {s}), mean {mean:.3} (pinned {m})")),
            None => failures.push(format!("{id}: not pinned")),
        }
    }
    assert!(compared >= 24, "the capture covers the SVG cases");
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
